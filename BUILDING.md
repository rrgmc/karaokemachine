# Building KaraokeMachine

Everything about compiling, testing, packaging and releasing. If you only want to *use* a karaoke
machine, [`README.md`](README.md) is the document you want — this one assumes you are going to build
one.

---

## Quick start

```sh
git clone https://github.com/rrgmc/karaokemachine
cd karaokemachine

tools/setup/fetch-assets.sh        # the GM SoundFont, once per machine
cargo build --workspace      # first build compiles SDL from source: ~2-3 minutes
cargo run -p karaokemachine
```

That is the whole thing on a machine that already has Rust and a C toolchain. The
[prerequisites](#prerequisites) below are what to install if it is not.

**There is no Makefile, and nothing to install to get one.** Everything routine is a `cargo` command.
The longer ones have aliases in the committed `.cargo/config.toml`, so `cargo km-test` is the same
thing as typing the full line, and neither is privileged.

The shell scripts under `tools/` are the jobs cargo has no business doing. Fetching things from the
network, building a Debian package in Docker, cross-compiling for Android, and starting three servers
to drive a browser for the screenshots.

**This table is every alias in `.cargo/config.toml`, and it is meant to stay that way.** A table that
lists two thirds of them sends somebody to `cargo run -p`, which is the spelling that file exists to
replace.

| Alias | Is exactly |
|---|---|
| `cargo km-build` | `build --workspace --features …` (the list is in `tools/setup/features.sh`) |
| `cargo km-test` | `test --workspace --features …` — the same list |
| `cargo km-lint` | `clippy --workspace --all-targets --features … -- -D warnings` |
| `cargo km-build-no-video` | the same three with video declined — `--features km-song/testing,km-api/testing` |
| `cargo km-test-no-video` | `test --workspace --features km-song/testing,km-api/testing` |
| `cargo km-lint-no-video` | the same list, through clippy |
| `cargo km` | `run -p karaokemachine --features video` |
| `cargo km-no-video` | `run -p karaokemachine` — no ffmpeg needed, and it says it cannot play video |
| `cargo km-stream` | `run -p karaokemachine --features video,tray -- --stream` — the **only** spelling that carries the icon in the bar; a plain `cargo km -- --stream` compiles it out |
| `cargo km-package-builder` | `run -p km-package-builder --` |
| `cargo km-package-builder-video` | …with `--features video` |
| `cargo km-pkgbuild` | the same as `km-package-builder`, for people who would rather not type all of that |
| `cargo km-pkgbuild-video` | …with `--features video` |
| `cargo km-pkgbuild-desktop` | …with `--features desktop`, the windowed build |
| `cargo km-package-simple` | `run -p km-package-simple --` |
| `cargo km-package-simple-video` | …with `--features video` |
| `cargo km-package-simple-desktop` | …with `--features desktop`, the windowed build |
| `cargo km-pack` | `run -p km-pack --` |
| `cargo km-pack-video` | …with `--features video` |
| `cargo km-lyrics` | `run -p km-lyrics --` |
| `cargo km-remote` | `run -p km-remote --` |
| `cargo km-remote-desktop` | …with `--features desktop`, the windowed build |
| `cargo km-carols` | `run -p km-carols --` |
| `cargo km-pick` | `run -p km-pick --` |
| `cargo km-preview` | `run -p km-display --example preview` |
| `cargo km-wallpapers` | `run -p km-display --example wallpapers` |
| `cargo km-icon` | `run -p km-display --example icon` |
| `cargo km-banner` | `run -p km-display --example banner` |
| `cargo km-screenshots` | `run -p km-display --example screenshots` |
| `cargo km-admin` | `run --manifest-path tools/cmd/assets/Cargo.toml -p km-admin --` |
| `cargo km-admin-desktop` | …with `--features desktop`, the windowed build |
| `cargo km-wallpaper-pack` | the same for `-p km-wallpaper-pack`, and **`--release`** — see below |
| `cargo km-build-assets` | `build` over that workspace |
| `cargo km-test-assets` | `test` over that workspace |
| `cargo km-lint-assets` | `clippy --all-targets -- -D warnings` over that workspace |
| `cargo km-fmt` | `fmt --all`; `cargo km-fmt --check` checks instead of applying |
| `cargo km-fmt-assets` | the same over the assets workspace — `task fmt` is these two |

Arguments append, so `cargo km -- --headless` and `cargo km-package-builder ./songs --init` both work.

**The `-assets` ones reach the other workspace.** `tools/cmd/assets/` is excluded from this one,
because its two members need TLS from `reqwest` and `km-package-builder` needs it to have none. `-p`
cannot see them, so anything that wants one has to name `--manifest-path`. That path is written once
in `.cargo/config.toml`, and `task fmt` forwards to it rather than keeping a copy. The qualifier goes
after the verb, as in `km-lint-no-video`. `km-lint-assets` lints the assets *workspace*, where
`km-admin-lint` would read as linting the one crate of that name.

**`km-lint` and `km-lint-assets` are the two aliases that will not take an extra flag.** Their
trailing `-- -D warnings` swallows whatever follows, so `cargo km-lint --locked` hands `--locked` to
clippy as a lint name. That is why `tools/platform/linux/check.sh` spells its own `--locked` clippy
and test runs out in full. It then *asserts* that the aliases carry the same feature list.

**The workspace commands are the video build and the per-tool ones are not**, which looks
inconsistent and is deliberate. `km-build`, `km-test`, `km-lint` and `km` carry video because that is
what this project is. `km-pack` and `km-pkgbuild` do not, because `km-pack book` and `km-pack check`
never open a song's bytes, and a MIDI or MP3+G corpus scans without ffmpeg. Those two ask for video
by name, as `cargo km-pack-video` and `km-pkgbuild-video`.

### `Taskfile.yml`, the one list of everything

There is also a [Taskfile](https://taskfile.dev), and it does not take the paragraph above back. It
is a table of contents for it. Every task wraps a `cargo` alias or a script from `tools/`, and the
wrapped thing stays the real one. What `task` adds is `task --list`, which prints all of them at once
with a sentence beside each. Neither a cargo alias file nor a directory of scripts can do that.

| Task | Does |
|---|---|
| `task build` | `cargo km-build`, video included; `task build:no-video` declines it |
| `task test` | `cargo km-test`, video included; `task test:no-video` declines it |
| `task lint` | `cargo km-lint` — clippy over every target, warnings denied, video included |
| `task fmt` | formats both workspaces: this one's members, and the excluded `tools/cmd/assets` |
| `task check` | `lint:local`, `lint:prose`, `lint:pin`, `lint:version`, `lint:mdns`, `lint:cargo`, `lint:labels`, `fmt:check`, `lint`, then `test` — the pass before a push |
| `task lint:local` | asserts no tracked file names a local path, address or person |
| `task lint:prose` | asserts the prose this branch adds, and the messages it commits them in, state the rule rather than narrating it, and take the sentence shape |
| `task lint:cargo` | asserts every value in `.cargo/config.toml` is a string, which is what a worktree can inherit without doubling it |
| `task lint:labels` | asserts every platform and program the bug form offers has a label in `tools/dev/labels.sh` |
| `task check:linux` | what CI's Linux job runs, in Docker, on this machine |
| `task dist` | stages every release this platform can carry — the machine, then all seven tools |
| `task run` | starts the staged machine, at whatever version this workspace is on, and hands the prompt back |
| `task run:package-builder` | the same for the curation tool; `-- /path/to/songs` is forwarded |
| `task run:remote` / `task run:assets` | the same for the other two products that are servers with a page |
| `task clean` / `task clean:old` | takes staged releases away; `:old` keeps the current version |
| `task verify` | installs the built `.deb` and unpacks the tarball, each in a clean container |
| `task deploy:linux HOST=user@box` | builds the `.deb`, sends it to the box over ssh, installs it and starts the service; `task deploy:linux:boot` is the once-per-box boot appearance |
| `task build:android` | cross-compiles both ABIs, stages them, and builds the debug APK |
| `task build:ios` | cross-compiles both slices, stages the assets, and compiles the `.app` — macOS only |
| `task assets` / `task ffmpeg` | the two once-per-machine fetches |

The two `run` tasks **do not wait for the app to exit**. `WAIT=1` is how you ask one to, and on
Windows `CONSOLE=1` blocks too, since it selects the executable that prints. Staging is quiet.
`VERBOSE=1`, or `-v` on any of the scripts directly, shows the build logs, and a step that fails
replays whatever it held back.

**`task` is optional and nothing here needs it**: no build, no test, no CI step, no release. Install
it (`winget install Task.Task`, `brew install go-task`) if you would rather have one list than
several. Everything in it is a command that works with `task` absent, which is the rule that keeps
the two from drifting apart.

---

## Prerequisites

Common to all three platforms:

| | |
|---|---|
| **Rust** | an exact version, via [rustup](https://rustup.rs). `rust-toolchain.toml` names it and pulls `rustfmt` and `clippy` in on first use, so there is nothing to select and nothing to keep updated — the first `cargo` command in the checkout downloads what the file asks for. `rustup update` is not part of building this. |
| **CMake** | SDL3 and SDL3_ttf are compiled from source (`build-from-source-static`), so their build needs it. |
| **A C/C++ toolchain** | For the same reason. Which one is platform-specific; see below. |

`rusqlite` is `bundled` and the synthesiser is pure Rust, so SQLite and a sound library are *not*
prerequisites.

**ffmpeg and libclang are, in practice.** Strictly they are not. A plain `cargo build --workspace`
still compiles the entire workspace without them, because the `video` cargo feature is off by
default. But every command in this file that carries no suffix is the video build: `cargo km-build`,
`cargo km-test`, `cargo km-lint`, `cargo km`, and `task check`. So a machine you intend to develop on
wants one more setup step, once:

```sh
tools/setup/fetch-ffmpeg.sh        # or: task ffmpeg
```

[Video songs](#video-songs) is what that does, and where it puts things. The `-no-video` and
`:no-video` twins in the tables above need none of it. They are for a session that reads a diff, or
that checks that the video-less build still compiles. See the `Video is the default build` decision
in `docs/decisions/song-sources.md` for why the short name is the video one.

### Windows

```powershell
winget install Rustlang.Rustup
winget install Kitware.CMake
```

The MSVC linker is what Rust's `x86_64-pc-windows-msvc` target uses. Visual Studio or the **Build
Tools for Visual Studio**, with the "Desktop development with C++" workload, already carries it. `cl`
does not need to be on `PATH`. Only the linker has to be findable, which the toolchain arranges.

Run the shell commands in this file from **Git Bash**, which ships with Git for Windows. They are
POSIX shell scripts; PowerShell will not run them.

### macOS

```sh
xcode-select --install                 # the Command Line Tools: clang, and libclang for later
brew install cmake
```

**One extra variable is needed on macOS for a hand-typed `cargo`**, and the failure without it looks
unrelated to this project:

```sh
export CMAKE_POLICY_VERSION_MINIMUM=3.5
```

SDL3_ttf vendors a FreeType whose `CMakeLists.txt` still says `cmake_minimum_required(VERSION 3.0)`.
CMake 4 dropped compatibility below 3.5 and fails the configure step, which surfaces as a
build-script panic naming neither CMake nor FreeType.

**Three of the four ways in already set it, so the export is narrower than it looks.** `Taskfile.yml`
sets it in a global `env:`, the release script `tools/platform/macos/app-bundle.sh` for the build it
drives, and `ci.yml` at the top level. What is left is a `cargo` command typed by hand, which is
exactly where the panic is hardest to place. [`docs/architecture/distribution.md`](docs/architecture/distribution.md)
has the boundary, and why an environment variable is safe for the Taskfile to carry when a feature
list is not.

### Linux

SDL needs the graphics and input development headers, and `cpal` needs ALSA. One script installs
them:

```sh
tools/platform/linux/apt-deps.sh            # what every build needs
tools/platform/linux/apt-deps.sh --video    # ...plus ffmpeg's dev libraries and libclang
```

It asks for sudo, or runs directly as root in a container. The list inside it is deliberately SDL's
own `README-linux` list rather than a minimal one. SDL's configure aborts on the *first* missing
dependency and names only that one, so trimming it costs a build per package to rediscover.

**That script is the only copy of the list.** CI runs it and `tools/platform/linux/Dockerfile` bakes
it in, so the three cannot drift apart.

---

## Building and testing

```sh
cargo build  --workspace
cargo test   --workspace --features km-song/testing,km-api/testing   # cargo km-test
cargo clippy --workspace --all-targets --features km-song/testing,km-api/testing -- -D warnings
cargo fmt    --all
```

**Why every command here names its features rather than saying `--all-features`.** Two crates keep
their test scaffolding behind a feature so it never ships in a release binary, and the integration
tests need both. Something has to be named either way.

**And on Linux `--all-features` cannot work at all**, which is why nothing here uses it. It turns on
the package builder's `desktop` feature, whose `wry` links libwebkit2gtk. A Linux build of that tool
is deliberately never given that library, because such a build would not start on a machine without
it. The one flag meaning "everything" therefore asks Linux for the one thing it will not provide.

The feature list lives once in `tools/setup/features.sh`. `.cargo/config.toml` spells it a second
time, because a cargo alias cannot source a shell file, and `tools/platform/linux/check.sh` asserts
the two agree. `cargo km-test` carries the video list, and `cargo km-test-no-video` carries the two
`testing` features alone. Do not write either out by hand.

Running the app:

```sh
cargo run -p karaokemachine                        # the app, in a window
cargo run -p karaokemachine -- --fullscreen        # ...filling the screen, this run only
cargo run -p karaokemachine -- --headless          # no window: API + engine + catalog
cargo run -p km-song --features testing --example write_fixtures    # once: writes fixtures/generated/
cargo run -p karaokemachine -- --play fixtures/generated/soft_karaoke_header_on_words_track.kar   # debug: play one file
cargo run -p karaokemachine -- --show-paths        # settings, catalog, packages folder, assets
cargo run -p karaokemachine -- --data-dir ./scratch         # keep a run out of the real install
cargo run -p karaokemachine -- --set-password hunter2       # change it (the /admin page and the API also do)
cargo run -p karaokemachine -- --reset-password             # back to a PIN, shown on the machine's screen
cargo run -p karaokemachine -- --reset-sessions             # sign every phone and browser out
cargo run -p karaokemachine -- -v                           # say more; -vv for everything
cargo run -p karaokemachine -- --frame-stats                # fps, frame times and decode, once a second
cargo run -p karaokemachine -- --list-audio-devices         # output devices and their stable ids
cargo run -p karaokemachine -- --song-book ./songbook.pdf   # every installed song, as a PDF to print
cargo run -p karaokemachine -- --song-book ./songbook.pdf --book-name "Sitting room"
```

The `--song-book` flag is an end-user feature and is described in
[`README.md`](README.md#the-song-book) rather than here.

The app is quiet by default: `info` and nothing per-frame. `-v` adds its own debug stream, `-vv` adds
everybody's, and `RUST_LOG` overrides both. `logging.level` in `settings.json` says the same thing in
`RUST_LOG`'s grammar, for a machine nobody types at.

`--frame-stats` is separate on purpose. It decides whether the frame rate is *measured*, which is not
a question about how much detail you want in a log. No log level turns it on, and turning it off
costs nothing. `KM_FRAME_STATS=1` does the same where there is no command line to pass it on.

**A checkout opens no mDNS daemon.** `.cargo/config.toml` sets `KM_NO_MDNS=1`, and `Taskfile.yml`
sets it for what it drives. `cargo km-test`, `cargo run` and `task check` therefore bind no multicast
socket, and Windows has nothing to raise a firewall dialog about.

`KM_NO_MDNS=0` in front of one command asks for mDNS back, which is what testing discovery from a
phone needs. The two tasks that launch a staged build clear it already, because running one of those
is how you watch the shipped behaviour. Nothing an owner installs reads the variable. The reasoning
is [`KM_NO_MDNS` declines the multicast socket, and one function honours it](docs/decisions/api-and-network.md#km_no_mdns-declines-the-multicast-socket-and-one-function-honours-it).

---

## Video songs

**Video is not something a person receiving this has to ask for.** Every
[staging script](#releases) builds it, on Windows, macOS, Linux and Android. Each stops with a
message where ffmpeg is missing, rather than quietly staging the lesser build. `--no-video` is the
opt-out, and it is deprecated. The flag and the build behind it stay supported, but nothing produces
one unless you ask.

What *is* optional is compiling it. Video sits behind a `video` cargo feature that is off by default
**in cargo**. There are three defaults here, which sound contradictory and answer three different
questions.

- **The cargo one is off.** It answers "what must somebody install before a fresh clone compiles at
  all". ffmpeg is the only C dependency in this project you have to install yourself, so
  `cargo build --workspace` needs neither it nor libclang. Untouched, and it is what keeps a clone
  cheap.
- **The release one is on.** It answers "what does somebody receiving a folder get to play", and
  there the answer is everything a catalog can hold. A build without the feature still catalogs,
  searches and queues video songs, and says at startup that it cannot play them.
- **The command one is on.** It answers "which name is short". `cargo km-build`, `cargo km-test`,
  `cargo km-lint`, `cargo km` and `task check` are all the video build, and declining it is spelled
  `-no-video`. That is not a preference about names. Alternating between the two feature sets
  invalidates every crate downstream of `km-video`, so the short name should be the one you actually
  want. See the `Video is the default build` decision in `docs/decisions/`.

So everything below is what building one needs, once. On a machine you intend to develop on, that is
a setup step rather than an optional extra.

One command, once per machine, on any of the three platforms:

```sh
tools/setup/fetch-ffmpeg.sh
```

- **Windows** — downloads one pinned, checksummed BtbN **LGPL shared** build into a cache outside the
  repository, and tells you to `winget install LLVM.LLVM` if libclang is missing.
- **macOS** — **builds** a pinned LGPL ffmpeg from source into that same cache, about a minute once.
  It builds rather than downloads, because nothing publishes a prebuilt *shared* LGPL macOS ffmpeg
  with headers, and because a [release](#releases) redistributes what it links. `--homebrew` takes
  Homebrew's GPL build instead, and libclang comes from the Command Line Tools you already have.
  **`brew install openh264` first**, because the pinned configure asks for it. That encoder is the
  one a streaming machine produces its picture with, and the only H.264 encoder an LGPL ffmpeg may
  carry. The script checks for it and stops before the build, rather than leaving the answer inside
  a configure log.
- **Linux** — installs `libav*-dev`, `libopenh264-dev` and `libclang-dev` through apt, dnf or
  pacman. Asks for sudo first and says so.

**The cache key is the ffmpeg version, so a prefix already in it is taken as it stands.** The pinned
build's *configure* flags are not part of that key. A change to them therefore reaches a machine that
has built before only when the directory goes, or the script takes `--force`. Nothing reports the
difference. The libraries are there, the build links them, and the absence shows up as a program
refusing to start a stream. Windows re-downloads on a changed pin and has none of this.

After that, **plain cargo builds video**, with nothing to export and nothing to source:

```sh
cargo km                                         # run the machine, video included
cargo km-build                                   # the workspace, with video
cargo km-test                                    # ...and its tests
```

That works because the script records the two paths in cargo's own `[env]` table, in
`$CARGO_HOME/config.toml`. It writes them between a pair of marker comments, and rewrites that block
in place. Cargo applies `[env]` to every build script it runs. The setting therefore reaches
`ffmpeg-sys-next` from any shell, from a script, and **from an IDE that invokes cargo itself**. None
of those inherit an `export` you typed. A real environment variable still wins, since cargo does not
mark these `force`, and that is how a hand-built ffmpeg takes their place.

> The paths cannot live in the project's committed `.cargo/config.toml`, because they are particular
> to one machine. Nor can a project-local override file: cargo's config `include` key is **silently
> ignored on stable** — verified on 1.98, where it neither errors nor takes effect — so such a file
> would look right and do nothing.

**Two things about this that are easy to get wrong**, both of which cost an afternoon here:

- `LIBCLANG_PATH` is **mandatory, not a convenience.** `ffmpeg-sys-next` runs bindgen at build time
  and ships no pre-generated bindings. Without it the build fails with `pkg-config` and vcpkg errors
  that do not mention clang at all.
- The pinned ffmpeg is **7.1.5, deliberately not the newest.** It matches what Debian trixie ships,
  which is what the appliance runs. A newer ffmpeg on a development box lets code compile against API
  Debian lacks, and that failure then appears only in CI.

Packaging video songs shells out to whatever `ffmpeg` is on `PATH`. That is a separate thing from the
libraries above, and it is needed only when a file falls outside the playback profile:

```sh
cargo km-pack-video spec ./songs --out vol1.kmspec.yaml
cargo km-pack-video build vol1.kmspec.yaml   # cargo km-pack-video build …
```

One profile covers every video: H.264 in 8-bit 4:2:0, at most 1080p30, AAC, MP4. A file that already
matches is copied byte-for-byte, which a `yt-dlp` download normally does.

### If your IDE fails with `ffmpeg-sys-next` errors

An IDE that builds the whole workspace compiles every member. RustRover does, and so does VS Code
with rust-analyzer set to `--workspace`. `km-video` is a member. Its ffmpeg dependency sits behind
**its own** default-off feature, so a bare workspace build compiles it to an empty library with no
ffmpeg needed.

An IDE that still fails is building with all features on. RustRover's Cargo settings have such a
switch, and rust-analyzer has `cargo.features`. Either turn that off, or run
`tools/setup/fetch-ffmpeg.sh` and let it stand. It records the paths in cargo's `[env]` rather than
in your shell. The IDE therefore picks them up on its next build, with nothing further to configure.
That is the practical reason to prefer the cargo config over exported variables: an IDE launched
from a desktop shortcut never saw your shell.

**The trade to know about.** With the feature off, `km-video` is an empty crate, so your IDE offers
no completion or error checking inside `crates/playback/km-video/src/lib.rs`. The file is
`#![cfg]`-ed out entirely. If you are working *on* the video decoder rather than around it, run
`tools/setup/fetch-ffmpeg.sh` once and turn the `video` feature on in your IDE. Everything else in
the workspace is unaffected either way.

---

## The other tools

The packaging and curation commands, `km-package-builder` and `km-pack`, are how a catalog gets made.
[`README.md`](README.md#getting-a-corpus-into-shape) documents them for their users. From a checkout
each has an alias in the table above, `cargo km-pkgbuild` and `cargo km-pack`, and those are the
spelling to use. A `cargo run -p` typed from memory is the one that comes out wrong.

What is left here is the part only a contributor runs.

```sh
# The HTTP API on its own, over an in-memory machine. Dev remote at /dev/.
cargo run -p km-api --example dev_server --features testing

# Every song read again and what the analysis says written back, then exit. What a change to
# km-suitability asks for: until this runs, every stored number describes the rubric before it.
# Minutes rather than the hour a forced scan takes -- one copy of each song, and none of the
# whole-corpus passes. *Re-analyze the songs* on the Scan page is the same work with a progress bar.
#
# Build it with --release first; a debug parse over a real corpus is not the same wait. And on
# Windows run the console twin from a prompt -- km-package-builder.exe is GUI-subsystem, so the
# shell does not wait for it and the flag refuses rather than detaching.
cargo km-pkgbuild <your karaoke folder> --reanalyze
target/release/km-package-builder-console <your karaoke folder> --reanalyze
```

Generated assets — wallpapers, the app icons, screen previews — are rebuilt from code:

```sh
cargo km-wallpapers        # the built-in gradient set (NOT the stock-photo pack)
cargo km-icon              # four program marks, and the machine's again with the stream badge
cargo km-banner            # the Android TV banner; needs icon-128.png first
cargo km-preview           # every screen to target/preview
tools/dev/screenshots.sh   # the eight pictures in docs/images that README.md shows
```

### The Christmas carol pack

Sixteen public-domain carols, built into one `.kmpkg` somebody downloads. It is **not bundled with
the machine**, and nothing about it touches `assets/` or any carrier. The [`A downloadable song
pack`](docs/decisions/repository.md#a-downloadable-song-pack) decision also says why this is the only
pack that could be built at all.

```sh
tools/dist/carols.sh          # or: task carols
tools/dist/carols.sh -v       # ...watching the conversion
```

It needs the network once, to fetch the source hymnal into the asset cache and verify it against a
pinned SHA-256. It also needs **`abc2midi`**, which nothing else here asks for:

```sh
task abcmidi          # or: tools/setup/fetch-abcmidi.sh
```

That builds it into the asset cache, once per machine, on any of the three platforms, and
`task carols` finds it there by itself. It needs an ordinary C compiler and nothing else: no
libraries and no build system. It picks one by **trying** `$CC`, `cc`, `gcc` and `clang` in turn,
rather than by guessing from the platform. If none of them works it says what to install.

It is built rather than downloaded, because **the project publishes no binaries**. No GitHub
releases, nothing in its SourceForge file area, and the site its own repository points at is gone.
A system package wins where one exists and is found first: `apt install abcmidi`,
`brew install abcmidi`. There is no winget package, which is the case the script exists for.

One thing to know on Windows: **a clang that targets MSVC cannot build abcMIDI.** It defines
`snprintf` as `_snprintf` behind an `_MSC_VER` guard, for the benefit of very old compilers, and a
modern UCRT header refuses that outright. A MinGW gcc has no `_MSC_VER`, never takes that branch, and
builds it in one line. `winget install BrechtSanders.WinLibs.POSIX.UCRT` is one.

`abc2midi` is a **build-time** tool on the same footing as `ffmpeg` the command and Inno Setup.
Nothing links it and nothing ships it, so its GPL reaches no released artifact. `--abc2midi <path>`
names one kept somewhere else.

---

`preview` and `screenshots.sh` are not the same job. The first is a diagnostic contact sheet of the
twenty states that are easy to get wrong. It draws them on a flat background, for looking at in a
pull request.

The second produces the handful of pictures that are *published*, over a real wallpaper and against a
real catalog. It therefore needs a folder of songs in `KM_CORPUS`, and without one it deliberately
writes to `target/screenshots` rather than touching what is committed. It keeps the package builder's
recent-folder list under `KM_SHOT_DIR` rather than in yours. That is the same reason it gives the
other two servers a `--data-dir` of their own.

---

## Releases

Each script stages a distributable and says what it produced.

**The Windows folder holds two executables, and only Windows does.** The windowed one, and the
`-console` twin, which is a diagnostic rather than the one to reach for. The
`The machine's console window` decision in
[`docs/decisions/distribution.md`](docs/decisions/distribution.md) says which is which, and why. No
end-user document names the twin. What matters here is that `tools/platform/windows/dist.sh` stages
both, and the installer stages only the first.

```sh
tools/platform/windows/dist.sh          # portable folder: two exes + assets + ffmpeg's DLLs
tools/platform/macos/app-bundle.sh      # Karaoke Machine.app, on macOS; finds ffmpeg and libclang itself
tools/platform/linux/deb.sh             # a Debian 13 .deb, built in Docker
tools/platform/linux/deb.sh --tools     # ...and karaokemachine-tools, the three curation tools
tools/platform/linux/verify-deb.sh      # install that .deb in a clean container (--tools for the other)
tools/platform/linux/tarball.sh         # a portable Linux folder + .tar.gz, built in Docker
tools/platform/linux/verify-tarball.sh  # unpack and run it in a clean container (--image to pick one)
tools/dist/cmd.sh            # all seven: km-pack, km-lyrics, km-package-builder,
                             #   km-package-simple, km-remote, km-admin, km-wallpaper-pack
tools/dist/cmd.sh km-package-builder  #   ...or just one of them
tools/dist/cmd.sh --no-video #   ...without the video feature; every script above takes this
tools/dist/bin.sh              # one folder with every executable in it, instead of one per product
tools/dist/bin.sh --no-build   #   ...gathering what is already staged, building nothing
tools/platform/windows/installer.sh     # a Windows setup.exe: one installer, all seven products, per-user
tools/platform/macos/installer.sh       # a macOS .pkg: one installer, all seven products, /Applications
tools/platform/windows/installer-remote.sh  # ...and the remote alone, about 5 MB, its own uninstaller
tools/platform/macos/installer-remote.sh    #   the same on macOS: KM Remote in /Applications, nothing else
task build:ios RELEASE=1 DEVICE=1 IPA=1         # an unsigned .ipa: the machine, for sideloading
task build:ios:remote RELEASE=1 DEVICE=1 IPA=1  #   ...and the offline remote
tools/dist/release.sh           # gather the carriers into dist/release/<version>/ under release names
tools/dist/release.sh --upload  #   ...and put them, and the body, on the draft GitHub release
tools/dist/release.sh --platforms windows,linux,android  # ...the carriers one machine builds
tools/dist/release.sh --upload --platforms windows,linux,android,ios --elsewhere macos
                                #   ...and a page that also names what a Mac adds
tools/dist/release.sh --add --platforms macos   # on the Mac: add its packages to that draft
task release:macos              #   ...both notarized packages built, then that, at the tag only
```

**`--platforms` is for the release no one machine can cut.** A Mac produces the two `.pkg` files and
the two `.ipa` files, and nothing else does. Naming the platforms a run carries therefore takes the
rest out of the table. It takes them out of the count, and out of the body's download table, at the
same time. A carrier of a platform that *was* named and is not staged still stops the run. The rule
is
[`A release page carries the platforms the machine cutting it can build`](docs/decisions/distribution.md#a-release-page-carries-the-platforms-the-machine-cutting-it-can-build).

**A pushed `v*` tag runs all of this in CI, except the Mac's half.** `.github/workflows/release.yml`
stages ten of the twelve carriers on hosted runners and runs
`release.sh --upload --platforms windows,linux,android,ios --elsewhere macos`. A Mac builds the two
`.pkg` files, and `release.sh --add --platforms macos` adds them. [`RELEASE.md`](RELEASE.md) has the
order.

The workflow needs the repository secrets `KM_ANDROID_KEYSTORE_B64`, the keystore base64-encoded,
and `KM_ANDROID_KEYSTORE_PASSWORD`. It also needs `KM_ANDROID_KEY_ALIAS` and
`KM_ANDROID_KEY_PASSWORD` where they differ from the defaults. See
[`CI builds the release, and a Mac adds its packages`](docs/decisions/distribution.md#ci-builds-the-release-and-a-mac-adds-its-packages).

**`tools/dist/release.sh` gathers and never builds**, for the reason `tools/dist/bin.sh --no-build`
exists. It names the command behind any carrier that is not staged, and stops, so a release is cut
from artifacts somebody has looked at. Its table is the one place an asset's published name is
written down, and the copy into `dist/release/<version>/` happens before `--upload` sends anything.
`--upload` creates a **draft** and fills it. Publishing is `gh release edit v<version> --draft=false`,
and a tag whose release is already published is refused rather than clobbered.

**The page's body is `tools/dist/release-notes.md`**, a tracked file written for somebody choosing a
download. See
[`What a release page says, and to whom`](docs/decisions/distribution.md#what-a-release-page-says-and-to-whom).
The run substitutes `@VERSION@` and the carol pack's name into `dist/release/<version>-notes.md` and
sends that, so the tracked file names no version. Edit its `What changed` list before a cut.
`--notes-file <path>` overrides it. Every `--upload` rewrites a draft's body, so a corrected sentence
is a re-run.

**Linux carries three**, and the third is `karaokemachine-tools`. That package holds the two package
builders, the offline remote and km-admin, and the machine Recommends it. It is a download beside the
machine's `.deb` rather than something `apt` fetches, there being no repository to fetch it from.
`task dist:deb:tools` stages it, and the release page names both files in one `apt install` line.

**The macOS carrier is the notarized package**, which is why that row names
`task dist:setup:notarized` and its pattern stops at the architecture. The signed-only and ad-hoc
builds sit in the same folder under names ending in `-unnotarized` and `-unsigned`, and the run
gathers neither.

**The two iOS carriers are unsigned, and say so in the name.** `--ipa` on either iOS build packages
the app it has just compiled into `dist/<product>/ios/`, refusing a debug build and, for the machine,
`--no-video`. There is no signed alternative: the decision is
[`An iOS carrier is unsigned, and the person installing signs it`](docs/decisions/distribution.md#an-ios-carrier-is-unsigned-and-the-person-installing-signs-it),
and [`README.md`](README.md#installing) is where a recipient is told how to sign one.

**On Windows there is also a setup program, and it is the one carrier that is not something you
unpack.** `tools/platform/windows/installer.sh` builds
`dist/setup/windows/karaokemachine-setup-<version>-windows-x86_64.exe`. That is one Inno Setup
installer, and it holds all seven products behind component checkboxes. Somebody who wants only the
offline remote therefore pays for neither the instrument bank nor the video libraries.

It installs **per-user** into `%LOCALAPPDATA%\Programs` and raises no UAC prompt. It offers to put
itself on your `PATH` and to open `.kmbuild` corpus files. It **leaves your songs, settings and
catalog alone when uninstalled**, and says where they are on its way out. It needs Inno Setup 6
(`winget install JRSoftware.InnoSetup`), which it looks for in the per-user location `winget` uses
before the Program Files ones.

The installed folder holds the **windowed** executable of each product and not the console twin. The
`What an installed build contains` decision in
[`docs/decisions/distribution.md`](docs/decisions/distribution.md) says why. There is no `--no-video`
installer. A payload with no ffmpeg in it stops the build, rather than producing a setup that lists
video songs and cannot read them.

**It also holds a `README.txt` that is not the payload's**, and it is the one file both setup
programs deliberately decline to install. The payload's copy describes the folder
`tools/dist/bin.sh` gathers, which an installed build is not. `dist_installed_readme` in
`tools/dist/common.sh` writes what gets installed instead, shared by the Windows and macOS
installers, so the two cannot describe removing the same product differently. Each names the
exclusion where its own carrier check would otherwise object: the payload-coverage check on Windows,
and `excluded()` on macOS. Both round trips read the installed file.

[`README.md`](README.md#installing) has what SmartScreen and Gatekeeper show the first person to run
an unsigned build, and how to find the macOS uninstaller. Those are what a recipient sees rather than
what a build does.

**macOS has one too, and it is an Apple installer package.** `tools/platform/macos/installer.sh`
builds `dist/setup/macos/karaokemachine-setup-<version>-macos-<arch>.pkg`. That is the name when the
package is notarized. A package that is not notarized takes `-unnotarized` or `-unsigned` before the
extension. It holds the same seven products behind the same component ticks. Somebody who wants only
the offline remote therefore pays for neither the instrument bank nor the video libraries.

The four applications go to **`/Applications`**, and the six command-line tools to
`/usr/local/karaokemachine`. Symlinks in `/usr/local/bin` point at those tools, and that directory is
already on your `PATH`. There is therefore no "add me to your `PATH`" tick, and nothing edits a
`~/.zshrc`. It asks for your administrator password once, for those two directories, and fetches
nothing at any point.

**System-wide, where the Windows one is per-user, and that is the same argument reaching the
opposite answer.** Everything the Windows installer configures beyond the files lives in that user's
registry. On macOS a bundle in `/Applications` declares the `.kmbuild` association, and the "`PATH`
entry" is a symlink in `/usr/local/bin`. Both of those belong to the machine.

**With a Developer ID it signs and notarizes**, and then it opens on an ordinary double-click:

```sh
task dist:setup                          # the ad-hoc build: a recipient right-clicks and picks Open
task dist:setup:notarized                # ...signed, notarized and stapled, the one you can hand over
task dist:setup:remote:notarized         # ...the remote's own package in the same state
tools/platform/macos/installer.sh --notarize     # the same thing without task
```

**The three values that needs are already in the script**, at the top of
[`tools/platform/macos/installer.sh`](tools/platform/macos/installer.sh)'s signing section. This
therefore takes no setup on the machine the certificates live on. To sign as yourself, either replace
them there, or override them without touching a tracked file. `security find-identity -v` lists what
your keychain actually holds. An environment variable is what an override is for:

```sh
KM_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
KM_SIGN_INSTALLER_IDENTITY="Developer ID Installer: Name (TEAMID)" \
KM_NOTARY_PROFILE=your-profile \
  tools/platform/macos/installer.sh --notarize
```

They are committed because **none of them is a secret**. The two identities are certificate common
names, which `pkgutil --check-signature` prints from any package signed with them. The profile is
only a keychain label. The Apple ID and app-specific password behind it stay in the keychain, and
`xcrun notarytool store-credentials` puts them there once. What they do carry is a person's name. The
[`What a committed file may say about the machine it was written on`](docs/decisions/repository.md#what-a-committed-file-may-say-about-the-machine-it-was-written-on)
decision therefore argues them under *Published identity*, rather than simply pasting them in.

There are two identities because there are two certificate types. Application signs the bundles, and
Installer signs the archive. Setting only the first is refused, because Gatekeeper judges the
archive.

Notarizing is a separate flag rather than part of signing. It goes to Apple and takes minutes. On
Catalina and later it is the half that removes the dialog. A Developer ID signature on its own is not
enough for a downloaded file. **The defaults apply on that flag alone**, so every other path is still
ad-hoc and unsigned. That is what keeps a clone with no Apple account able to build a release.

**Never combine it with `--no-build`.** Signing happens while the payload is staged. Skipping the
staging therefore leaves an ad-hoc payload inside a signed wrapper, and only the round trip at the
very end notices.

**Android has one variable and the same shape.** `KM_ANDROID_KEYSTORE` names the release key. Without
it the debug key signs the release build, so a fresh clone builds an APK with nothing set up:

```sh
KM_ANDROID_KEYSTORE=/path/to/release.jks \
KM_ANDROID_KEYSTORE_PASSWORD=... \
  task build:android RELEASE=1
```

`KM_ANDROID_KEY_PASSWORD` falls back to the store's, which is what PKCS12 requires them to share, and
`KM_ANDROID_KEY_ALIAS` defaults to `karaokemachine`. **A keystore named but not there stops the
build** rather than falling back, and every release build prints which key it used.
`tools/port/apk-signer.sh <apk>` asks the same question of a file, and `tools/dist/release.sh` refuses
to publish a debug-signed APK. See
[`How the Android applications are signed`](docs/decisions/remotes.md#how-the-android-applications-are-signed).

The four Docker-driven scripts above build their images on demand. The first of them on a new machine
is therefore slow, and the rest are not. `tools/platform/linux/prewarm.sh` does that up front
instead, which is worth running once if you would rather pay it deliberately. `--check` reports what
is cold without building anything. It is never required — every script still builds what it needs if
it is missing.

**All of them build video by default**, and stop with a message naming
[`tools/setup/fetch-ffmpeg.sh`](#video-songs) rather than quietly staging a lesser build if it is
missing. `--no-video` opts out, and the *declined* build is the one that gets a marker in its name.
The plain name should name what the plain command produces. (`tools/dist/cmd.sh km-lyrics` needs no
ffmpeg at all: nothing it was asked for has such a feature, so nothing checks for one.)

Everything lands under `dist/<app>/<platform>/`:

```
dist/karaokemachine/windows/karaokemachine-1.18.0-x86_64-pc-windows-msvc/   (+ .zip with --zip)
dist/karaokemachine/windows/karaokemachine-1.18.0-x86_64-pc-windows-msvc-no-video/
dist/karaokemachine/macos/Karaoke Machine.app
dist/karaokemachine/linux/karaokemachine_1.18.0-1_amd64.deb
dist/karaokemachine/linux/no-video/karaokemachine_1.18.0-1_amd64.deb
dist/karaokemachine-tools/linux/karaokemachine-tools_1.18.0-1_amd64.deb
dist/karaokemachine/linux/karaokemachine-1.18.0-x86_64-unknown-linux-gnu/   (+ .tar.gz)
dist/km-pack/windows/km-pack-1.18.0-x86_64-pc-windows-msvc/
dist/km-lyrics/windows/km-lyrics-1.18.0-x86_64-pc-windows-msvc/
dist/km-package-builder/windows/km-package-builder-1.18.0-x86_64-pc-windows-msvc/
dist/setup/windows/karaokemachine-setup-1.18.0-windows-x86_64.exe
dist/setup/macos/karaokemachine-setup-1.18.0-macos-aarch64.pkg              (notarized)
dist/setup/macos/karaokemachine-setup-1.18.0-macos-aarch64-unnotarized.pkg  (signed only)
dist/setup/macos/karaokemachine-setup-1.18.0-macos-aarch64-unsigned.pkg     (ad-hoc, the default)
dist/setup/windows/km-remote-setup-1.18.0-windows-x86_64.exe
dist/setup/macos/km-remote-setup-1.18.0-macos-aarch64.pkg                   (the same three signing states)
```

The `.deb` gets a subfolder rather than a suffix. `cargo-deb` names the file from the package and the
version, and a cargo feature changes neither. Two builds in one directory would therefore overwrite
each other.

**The macOS setup program is named for its signing state on the same rule.** The marker goes on the
declined build. The notarized package is the only one worth handing anybody, and it keeps the plain
name. There are three names because there are three states, and the middle one is not a rounding
error. `spctl` refuses a signed-but-unnotarized archive exactly as it refuses an unsigned one.

They coexist rather than overwriting, so rebuilding ad-hoc for a local test cannot destroy a
notarized package. `clean:old` is unaffected. It reads the version as the field after the app name,
and still finds it in a marked name.

App first, then platform, because a release is a thing you hand to somebody. You want every build of
one product together, not every Windows build of everything. The rule and the idioms the staging
scripts share live in `tools/dist/common.sh`.

`tools/dist/bin.sh` answers the other question, and is the one exception to the rule above:

```
dist/bin/<platform>/           every executable this platform can build, windowed where there is a
                               choice, with the assets and libraries they need beside them
dist/bin-console/<platform>/   the same, taking the console form of anything that has one
```

**On macOS the `.app` is the windowed form and the bare executable is the console one.** Windows
spells that same pair as `km-remote.exe` beside `km-remote-console.exe`. So `dist/bin/macos` holds
`KM Remote.app` and `dist/bin-console/macos` holds `km-remote` — one form in each folder,
not both in both.

Executables only — no `.deb` and no installer — and it *gathers* rather than builds. It runs the same
staging scripts as everything above, and copies what they produced. The two therefore cannot disagree
about what a release contains. `--no-build` skips straight to the copying. The folders carry no
version and `--zip` produces one that does.

`tools/platform/linux/check.sh` runs CI's Linux job in the same Debian container: fmt, clippy and the
whole test suite. It takes about a minute against a warm cache. It is worth running before a push
from Windows. Three failures appear only off Windows: a `Path` treating `\` as an ordinary character,
a missing system library, and a wrong `#[cfg]`. All three have happened.

**All three desktops build video, and each pays for it differently.** All three of them travel.
Windows is the easy case. `tools/platform/windows/dist.sh` stages the four ffmpeg DLLs the executable
actually imports beside it, with their LGPL terms. It then proves the folder is self-contained, by
starting it with ffmpeg stripped from `PATH`. The folder therefore copies to a USB stick and runs.

**Both Linux carriers carry the same built ffmpeg.** `tools/platform/linux/deb.sh` puts it in
`/opt/karaokemachine/lib`, and the package `Depends` on no `libav` at all. That is what lets an
appliance install decline X — see
[`An appliance install carries no display-server stack`](docs/decisions/distribution.md#an-appliance-install-carries-no-display-server-stack).
`deb.sh --system-ffmpeg` builds the other trade, linking Debian's and taking X with it, which is what
a distribution packager wants.

**The tarball has no alternative**, because a folder can name no dependency. Debian's ffmpeg cannot
travel inside one. It is GPL-2+ by Debian's own copyright file, while this workspace is
`MIT OR Apache-2.0`. It also drags 93 shared libraries and 97 MiB behind it.
`tools/platform/linux/ffmpeg-lgpl.sh` builds the same pinned LGPL ffmpeg the macOS branch does,
inside the build container. The tarball carries those four at about 13 MiB, linking nothing but libc.

**macOS costs the most work**, because a Mach-O names its libraries by the absolute path it was
linked at. `tools/platform/macos/app-bundle.sh` copies the four libraries *and the nine more they
pull in* into `Contents/Frameworks`. It rewrites every load command to `@rpath`, and re-signs each
file. `install_name_tool` invalidates the ad-hoc signature these carry, and dyld refuses to load a
library whose signature does not match. It then asserts that nothing loads by absolute path.
`tools/dist/cmd.sh` does the same for the km-pack and km-package-builder folders, into a `lib/`
beside the executable.

All three are written up in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

Bundling means **redistributing ffmpeg**, which is why `tools/setup/fetch-ffmpeg.sh` builds a pinned
LGPL ffmpeg from source on macOS rather than taking Homebrew's. Homebrew's is the GPL configuration:
`--enable-gpl`, with x264 and x265. This application never encodes through those libraries at all.
Linking it therefore ships a GPL build carrying 25 MB of encoders nothing can call. The source build
takes about a minute, once per machine, and produces four dylibs and 15 MB that depend on nothing
outside macOS. `--homebrew` takes Homebrew's instead, and the license note staged beside the
libraries works out which case it is in from what was actually copied.

Its container carries ffmpeg too, so `check.sh` covers the `video` feature rather than only the part
with no C dependency.

**CI runs every pull request on Linux, Windows and macOS**, with the video build on Linux in
`debian:13-slim`, and `master` requires its `CI ok` check. What each job runs, and why it is one
workflow, is [`What CI runs`](CONTRIBUTING.md#what-ci-runs) in `CONTRIBUTING.md`.

---

## Layout

| Crate | Owns |
|---|---|
| `crates/song/km-song` | Parsing MIDI/KAR, lyric extraction, encoding detection |
| `crates/song/km-suitability` | Melody detection and the 0–10 suitability |
| `crates/song/km-kmpkg` | The `.kmpkg` container |
| `crates/song/km-catalog` | The SQLite catalog |
| `crates/playback/km-audio` | Synthesiser and sequencer |
| `crates/playback/km-video` | Video decoding — **the only crate that names ffmpeg** |
| `crates/machine/km-api` | HTTP + WebSocket control surface |
| `crates/playback/km-display` | The SDL3 user interface |
| `crates/remote/km-remote-pages` | The singer's remote — one template set, two modes. Linked into `km-app` for the reduced remote the machine serves at `/`, and into `km-remote` for the full one |
| `crates/remote/km-remote-core` | The offline remote as a library — the catalog mirror, the favorites, the machine client, discovery and the server. No command line, no data-directory guess, nothing that prints, no signal handler: the desktop, Android and iOS shells each supply those |
| `crates/remote/km-remote` | The desktop shell over it: the standalone offline remote, in a window of its own on Windows and macOS |
| `crates/platform/km-tray` | The icon in the OS icon bar for a tool that runs a web server — so a run with no window is still visible, and can still be closed. Shared by `km-package-builder` and `km-remote` |
| `crates/machine/karaokemachine` | The binary |
| `tools/cmd/km-pack` | Packaging, as a library *and* a command |
| `tools/cmd/km-lyrics` | Dump a parsed lyric timeline and analysis for one file, or scan a folder |
| `tools/cmd/km-package-builder` | The curation web tool: a folder of source files in, `.kmpkg` packages out |
| `tools/cmd/km-package-simple` | The folder packager: one folder in, uncurated `.kmpkg` packages out, with no database |
| `tools/cmd/assets/km-wallpaper-pack` | Builds a legibility-verified wallpaper pack. **In the second workspace** — `tools/cmd/assets` is `exclude`d from this one, see the note in `Cargo.toml` |

---

## Android

```sh
task ffmpeg:android      # once per machine: the LGPL ffmpeg, both ABIs
task build:android       # cargo cross-compiles both ABIs, Gradle packages -> app-flat-debug.apk
task build:android:quest # the headset APK, from the same native libraries
```

[`ports/machine/android/README.md`](ports/machine/android/README.md) is the full account. It says why
there is no `externalNativeBuild` block anywhere in the project, and what is copied from SDL and what
is ours. It says why `minSdk 26`. It also says why `JAVA_HOME` must name a JDK 17 or 21, rather than
the JBR that ships with Android Studio.

`RELEASE=1`, `ARM64=1` and `NO_VIDEO=1` reach the underlying scripts. `ARM64=1` is for a quick
phone-only iteration, and is **not** a thing to ship. Every Google TV device runs a 32-bit OS and
loads `armeabi-v7a` alone. An APK without it installs on a phone and fails on a television.

**The two commands are two product flavours of one Gradle project**, `flat` and `headset`, and they
share the native libraries and the assets. So a headset APK costs a second `assemble` and no second
cross-compile. The headset one takes `arm64-v8a` alone, needs Android 14, and installs beside the
flat one under its own application id.


---

# Notes from building it

What was learned setting these toolchains up, kept because each item cost real time to find. The
sections above are what to do; these are why.

## Build environment

- Toolchain pinned by `rust-toolchain.toml` to an exact version, edition 2024, resolver 3. The
  workspace's `rust-version` is kept equal to it; see [Bumping the Rust toolchain](#bumping-the-rust-toolchain).
- **Dependencies are opted into per crate.** The root manifest carries the full version table, but a
  member lists only what it needs. Nothing is therefore compiled until it is used, and **each crate's
  manifest shows its real footprint.**
- `unwrap`/`expect`/`panic` are **not** denied workspace-wide, because tests use them legitimately and
  CI runs `-D warnings`. They are denied locally in the real-time audio modules, where they matter.
- `missing_docs` is warned workspace-wide and `unsafe_code` denied.
- **One dependency comes from git rather than crates.io** — `rustysynth`, for SF2 modulators,
  lenient bank loading and the pitch path; the reasoning is
  [`The synthesizer is a fork`](docs/decisions/audio.md#the-synthesizer-is-a-fork). Nothing about a
  build changes. `Cargo.lock` pins the commit exactly as a registry checksum does, and the fork's
  repository is public, so no credential is involved. A first build has to reach github.com as well
  as crates.io.

To try a change to the fork **before pushing it**, point cargo at a local checkout for one command.
Do not write this into `.cargo/config.toml`, which is tracked and may not name a local path:

```sh
cargo build --config 'patch."https://github.com/rrgmc/rustysynth".rustysynth.path="<your rustysynth checkout>/rustysynth"'
```

**It rewrites `Cargo.lock` on the way through**, and drops the `source` line. The entry then names a
bare `rustysynth 1.5.0` with no origin at all. That is a lock file that pins nothing, and `--locked`
rejects it everywhere. Put it back with a plain `cargo check` once you are done.
`git checkout -- Cargo.lock` restores the *previous commit's* lock instead, which is a different
thing where the dependency line itself is what you are changing.

**How to read a wall-clock figure in this document.** The older timings were taken with the checkout,
`target/`, the corpus, a large database and Docker's data disk all on one 7200rpm drive. Treat them as
**upper bounds on a contended machine**, not as what a build costs today. They are left as measured
rather than re-taken, because a figure with a known configuration is worth more than a fresh one with
neither.

## `task check` leaves a binary with no icon in the bar

**`target/debug/karaokemachine` after a `task check` is a build with no `tray` feature**, because that
pass names `KM_FEATURES_VIDEO` and `tray` is deliberately not in it. `cargo test --workspace` builds
the binaries too. A build with the icon compiled out therefore replaces the file a `cargo km-stream`
left there. A streaming run from it is silent about the difference, the icon being a blemish rather
than a fault.

**So rebuild before looking at the bar**, whenever a check has run since:

```sh
cargo km-stream          # or: cargo build -p karaokemachine --features video,tray
```

The same applies to anything staged out of `target/debug`. A bundle assembled by hand from a binary
a check wrote has no icon in it, whatever the source says.

## What `task check` costs

Measured on this repository's Windows box, over a tree that is already built and has no source change:

| Stage | Warm | Cold |
|---|---|---|
| `lint:local` | 0.6s | 0.6s |
| `lint:prose --changed` | 3.3s | 3.3s |
| `lint:pin`, `lint:version` | 0.2s each | 0.2s each |
| `fmt:check` | 3.1s | 3.1s |
| `lint`, root workspace | 0.9s | 73s |
| `lint`, `tools/cmd/assets` | 0.9s | 65s |
| `test`, root workspace | 25s | 116s |
| `test`, `tools/cmd/assets` | 10s | 139s |
| **the whole pass** | **43s** | **434s** |

- **The tests are the pass.** Of the 25s warm, 16s is inside the harness — 2872 tests over 87
  binaries — and the rest is starting them one after another.
- **A source change adds compilation on top.** Touching a leaf crate costs 12s in the test build;
  touching `km-song`, which 31 of the 35 members depend on, costs 36s.
- **`tools/cmd/assets` costs out of proportion to its 271 tests.** It builds at `opt-level = 1` with
  its dependencies at 2, and it shares no artifacts with the root build. Touching one of the nine
  `crates/` that `km-admin` depends on therefore costs 51s of recompilation there, on top of the root
  workspace's own.
- **Clippy and the test build share nothing.** Whichever runs second pays its own full traversal, so
  the two orders cost the same 200s cold. Dependencies are 62% of that compilation, members 38%, so
  no selection among members reaches most of it.
- **The whole-tree form of `check-prose.sh` is 176s**, and it is the audit rather than the gate.
  `task check` runs `--changed`, and `--commits` beside it reads a branch's own messages in under a
  second. The sentence shapes cost one `awk` per converted document, which is about a second of the
  176. The fourteen phrase shapes are the rest.

`Why the pass checks everything` in [`docs/decisions/repository.md`](docs/decisions/repository.md) is
what these figures decide.

## Bumping the Rust toolchain

The pin lives in `rust-toolchain.toml` and is the only place the version is *decided*. Three tracked
files have to repeat it, because no format lets them derive it. `tools/dev/check-toolchain-pin.sh`,
which `task check` runs, fails naming any that disagree:

| | |
|---|---|
| `rust-toolchain.toml` | `channel`. The pin itself, and an exact `x.y.z` is asserted. |
| `Cargo.toml` | `rust-version` under `[workspace.package]`. |
| `tools/cmd/assets/Cargo.toml` | its own `rust-version` under `[workspace.package]` — `tools/cmd/assets` is a second workspace, `exclude`d from the one above, so it inherits nothing. The easiest one to miss. **Its members inherit from it**, so this is one line and not one per crate. |
| `docs/learning-rust.md` | one sentence naming `rust-version`. |

```sh
rustup toolchain install --no-self-update    # installs whatever the file now says
tools/dev/check-toolchain-pin.sh             # the four above agree
task check                                   # and the workspace still builds clean on it
```

Four things worth knowing before you do it:

- **`rustup target add` is per toolchain, and a bump silently loses the targets.** Android and iOS
  standard libraries attach to the toolchain they were installed for, not to the checkout. The first
  Android build after a bump therefore fails its target check in
  `tools/port/machine/android/build.sh`. The fix is the line that script already prints — run it from
  inside the checkout and it lands on the new pin.
- **Clippy is the reason to run `task check` rather than trusting the build.** A new stable adds
  lints, `cargo km-lint` is `-D warnings`, and raising `rust-version` un-suppresses the MSRV-gated
  lints as well. Both land in the same commit as the bump, which is the entire point of pinning.
- **The Debian image rebuilds itself.** `tools/platform/linux/image-tag.sh` hashes
  `rust-toolchain.toml` into the image tag. A bump therefore produces a new tag, and the next
  `deb.sh` or `check:linux` re-runs rustup and `cargo install cargo-deb` — several minutes, once.
  Without that hash the old tag would still name an image with the old compiler baked in.
- **CI needs no edit.** The three workflows install with `rustup toolchain install --no-self-update`,
  which resolves the file. `dtolnay/rust-toolchain` cannot be used here. Its `toolchain` input is
  required and it does not read `rust-toolchain.toml`, and the checker fails if it comes back.

## Bumping the version

**One number covers every program in the repository**, and it is written down in exactly two places.
The reason is the toolchain pin's: `tools/cmd/assets` is a second workspace, `exclude`d from the one
above, and nothing is inherited across that boundary.
`tools/dev/check-version-pin.sh` — which `task check` runs — fails naming either if they disagree:

| | |
|---|---|
| `Cargo.toml` | `version` under `[workspace.package]`. |
| `tools/cmd/assets/Cargo.toml` | its own `version` under `[workspace.package]`, covering `km-admin` and `km-wallpaper-pack`. The easy one to miss. |

Every crate under either takes it with `version.workspace = true`, so the cargo half of a bump is
those two lines and the two lockfiles. The checker also refuses a crate that goes back to naming its
own.

```sh
cargo update --workspace                                                # root lockfile
cargo update --workspace --manifest-path tools/cmd/assets/Cargo.toml    # the other one
tools/dev/check-version-pin.sh                                          # the two agree
```

**`--workspace` moves the members and nothing else**, so a bump does not drag third-party crates
along with it. `cargo metadata --no-deps` will not do this job: `--no-deps` skips resolution, so it
writes no lockfile at all and reports success having changed nothing.

**Never substitute the old number through a lockfile.** A third-party crate sitting at the version
being replaced comes out pinned to one that does not exist.

**The changelog's `Unreleased` heading is retitled in the same commit**, to the new number and the
day. [`RELEASE.md`](RELEASE.md) is the order the whole cut goes in, and that is its first step. The
range it is written from is easiest to read before the tag exists.

Two things worth knowing:

- **Commit both lockfiles with the bump.** `tools/platform/linux/check.sh` and the release paths pass
  `--locked`, so a lockfile still naming the old version fails there rather than here.
- **The version is a folder name before it is anything else.** A staged folder is
  `dist/<app>/<platform>/<app>-<version>-<triple>`, named from the version the *binary* reports and
  found again using the version a *manifest* reports. That is why the two assets programs do not
  version independently. On numbers of their own, every script walking `dist/` needs an arm per
  program. The one `tools/dist/bin.sh` lacked staged `km-admin-0.1.0-…`, and then said nothing was
  staged for km-admin. See `One version number for the whole repository`
  in `docs/decisions/repository.md`.

### Seven more places, and nothing checks them

The two manifests are the half a script can verify. The number is also written by hand outside cargo,
where no check and no CI step will catch a miss:

| | |
|---|---|
| `ports/machine/android/app/build.gradle` | `versionName`, and `versionCode` up by one |
| `ports/remote/android/app/build.gradle` | the same pair, on a counter of its own |
| `ports/machine/ios/project.yml` | `CFBundleShortVersionString`, and `CFBundleVersion` up by one |
| `ports/remote/ios/project.yml` | the same pair, on a counter of its own |
| `BUILDING.md` | the sample staged paths under [Releases](#releases) |
| `tools/platform/windows/installer.iss` | the sample `ISCC.exe` line in its header comment |
| `tools/platform/windows/installer-remote.iss` | the same, three lines of it |

**Read each build counter out of its own file**, not off the last release commit — an intermediate
build may have spent one.

`git grep -F <old version>` afterwards is the check that exists. It finds prose too. A kernel version
in a sample journal line, a date-stamped record in an architecture note and a test fixture are all
coincidences. None of them must move.

**`CHANGELOG.md` is the deliberate one.** Every number in it is a record of a release that happened.
A substitution through that file therefore rewrites history rather than the version.

## Building on macOS

| Needed | Why |
|---|---|
| Command Line Tools — `xcode-select --install` | Apple clang and the SDK. cpal binds CoreAudio straight from it, so the ALSA/PulseAudio/X11 list Linux needs has **no macOS counterpart**: there is nothing to `brew install` for audio |
| `cmake` — `brew install cmake` | SDL3 is compiled from source rather than found on the system |
| `ninja` | not required (CMake defaults to Makefiles here) but faster, and Android needs it anyway |

**Why a Rust build needs CMake.** `build-from-source-static` means `sdl3-sys` builds SDL3 itself,
rather than looking for a system copy. SDL3 is a C library whose build system is CMake. SDL3_ttf then
does it a second time, for its vendored FreeType. The payoff is a self-contained binary, and no
`brew install sdl3` for anyone who builds this. The price is CMake plus a C toolchain on every
desktop platform.

**CMake 4 breaks the vendored FreeType**, which still declares a minimum of 3.0 — and CMake 4 removed
compatibility with anything below 3.5. Taking its own advice, via the environment so it reaches the
`cmake` crate's invocation:

```sh
CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo build --workspace
```

The variable is harmless when nothing needs it. It is set in three places, for the three ways a build
starts:

- the workflow's top-level `env:`, because the runners ship CMake 4 on all three platforms;
- the Taskfile's global `env:`, so `task check` on a Mac needs no prefix;
- the hand-typed `cargo` above, which is the only place it is spelled out.

**SDL needs more X11 development packages than an obvious list carries**, and the lesson is in how it
fails. SDL's configure aborts on the *first* missing required dependency, and names only that one.
Fixing them as they surface therefore costs a full CI round trip each. **Install SDL's whole
documented list, not the package the error names** — "SDL builds" is the property needed, and SDL
decides what that takes. Names are verified against the distribution's archive before being added,
because a typo fails the apt step instead, which looks like an unrelated problem.

**The toolchain cannot be stale here.** `rust-toolchain.toml` names an exact version and rustup
installs it on the first `cargo` command, so a checkout cannot be on the wrong compiler. A floating
channel fails before compiling a single crate, with an error naming dependencies rather than the
compiler. See [Bumping the Rust toolchain](#bumping-the-rust-toolchain) for the other side of that
trade.

### Staging a bundle asks the terminal for App Management

**Every `.app` staged here is signed, and macOS lets a program write inside a signed bundle only
where it holds App Management permission.** The permission is read against the application the
terminal belongs to, not against the shell. An editor's integrated terminal carries the editor's
grant, and a terminal application carries its own. So `task dist:bin` and the setup drivers go
through in one window and stop in another. The line they stop on names a path rather than a
permission:

```
mkdir: dist/karaokemachine/macos/Karaoke Machine.app/Contents: Operation not permitted
```

**Nothing prompts for it.** The request is refused where it stands, so the error above is the whole
of the symptom. Grant the application under System Settings, Privacy & Security, App Management, and
quit and reopen it — the grant is read at launch. Running the same command from an application that
already holds it works equally well.

`dist_stage_macos_bundle` asks the question before it acts on the answer: it makes and drops a
directory inside the bundle, and stops on a refusal. Staging clears a bundle in place, so without
that probe a refused run empties the staged `.app` and then cannot rebuild it.

### Two faults only a Mac shows

Both are macOS-only, and **the checks that actually run catch neither**. They have no root cause in
common. What they share is the shape. *The platform makes an assumption false, and the code that
depends on it reports the opposite of the truth rather than failing loudly.*

**Canonicalising needs the file to exist.** A check compared a tidied path against a tidied temporary
directory, and `tidy` canonicalises with a fall back to the raw path. The temporary directory exists,
so it canonicalised. The package being asked about does **not** exist, because an earlier branch
returns when it does, so it did not. On macOS `/var` is a symlink to `private/var`, so the two sides
matched nothing.

The consequence is in the branch behind it. **A package installed from the scratch directory, and
then swept away by the operating system, can never be forgotten.** The install-time guard at the
other call site is unaffected. *There* the file exists, so both sides canonicalise. **That is what
makes the answer depend on when it is asked.** The test pins both an existing and a vanished path,
because one of them passing proves nothing about the other.

**Rust panics on a broken pipe, so a verifier can fail its own release.** Piping `--show-paths` into
`grep -q` closes the pipe the moment it matches, while the machine is still writing. **Rust ignores
`SIGPIPE`, so `println!` gets `EPIPE` and panics**, and the process exits 101. Under
`set -o pipefail` the pipeline reports the 101 rather than grep's 0, **so the assertion fails
precisely when it matches.**

Deterministic, not a race: it needs three more lines of output after the first match, so it arrives
the moment `--show-paths` gains a line. **The instructive case is an assertion that fails the build
*when* a match is found.** The panic leaves it permanently unable to see the thing it exists to
catch.

**The general rule: never pipe one of this project's binaries into `grep -q` under `pipefail`.**
Piping from `find`, `otool` or `pkgutil` is safe by luck rather than by design. They die quietly on
`SIGPIPE`, and their output is short enough to be written before grep exits. A Rust program turns
the same shape into a panic and a false verdict. Capture into a variable and match afterwards.

## Task's `dir:` does not reach a spawned `cmd`

Running Gradle through `cmd /c gradlew.bat` with a `dir:` set fails on Windows with
`'gradlew.bat' is not recognized` **from a directory where `ls gradlew.bat` finds it**. A `.bat` is
not an executable image, so only `cmd.exe` can start one, and `cmd` is handed the *process's* working
directory rather than the task's. Measured in a scratch Taskfile: one path segment fails exactly as
three do, so depth is not the variable.

Linux and macOS never see it, because `gradlew` there is a shell script Task runs itself and `dir:`
does apply.

**The general rule: invoke bash through `{{.SH}}`, never `cmd`.** The one honest exception is
launching a staged binary, because `start` really is a `cmd` builtin.

## Building `km-video`

Two environment variables, and **both** are required. Without either, the build fails in ways that
look unrelated to it:

- `FFMPEG_DIR`, a directory holding ffmpeg's `include/` and `lib/`.
- `LIBCLANG_PATH`, **mandatory rather than a convenience**, because `ffmpeg-sys-next` runs bindgen at
  build time and ships no pre-generated bindings.

**Nobody exports either by hand.** `fetch-ffmpeg.sh` installs both and writes the paths into cargo's
own `[env]` table, between markers it rewrites in place. Recording it *there* is what elimination
leaves:

- Cargo applies `[env]` to every build script it runs. The setting therefore reaches the build from
  any shell, from a script, **and from an IDE that invokes cargo itself**. An IDE launched from a
  desktop shortcut never saw an `export`. "Works in my terminal, fails in the IDE" is the normal
  shape of this failure.
- The committed `.cargo/config.toml` cannot hold them: the paths are particular to a machine.
- **A gitignored project-local override file cannot either.** Cargo's config `include` key is
  **silently ignored on stable** — verified: a missing include does not error, and a *present* one has
  no effect. **A file that looks right and does nothing is worse than no file.**
- Cargo does not mark these `force`, so a real environment variable still wins — which is how a
  hand-built ffmpeg gets used instead.

**One thing `[env]` cannot do, and it bites on Windows only.** It can set a variable but not *append*
to `PATH`. So the DLLs go on the user's own `PATH`, which the script does once through PowerShell's
API. It never uses `setx`, which truncates at 1024 characters. It never echoes `$env:PATH` back to
User scope either, because that is the merged machine and user value. Doing so would copy the system
`PATH` into the user's permanently.

**Until a shell is restarted, a video build compiles and then dies at run time** with
`STATUS_DLL_NOT_FOUND` — observed, not predicted. Nothing is wrong with the build; the test binary
simply cannot find the DLL.

**On Linux neither variable is set, deliberately.** pkg-config finds the libraries, and clang-sys
finds libclang through the distribution's own layout. Setting either would replace a correct answer
with a guess. **On macOS neither has to be set by hand.** The bundle script resolves both itself, and
fails naming the missing one rather than letting bindgen fail obscurely.

**The crate version does not pin the ffmpeg version.** `ffmpeg-sys-next`'s build script reads the
installed headers, and enables version cfgs from a table spanning ffmpeg 3.0 to 9.0. One crate
version therefore compiles against 7.1.5 and against anything newer. **The obvious assumption — that
crate 9.x needs ffmpeg 9.x — is wrong, and believing it would send the whole dependency choice the
wrong way.**

**Develop against the version the appliance has, not the newest.** A newer ffmpeg on a development
machine lets code compile there against API Debian lacks, and the failure then appears only in Docker
or CI. **The development box wants to be the lower bound.**

### `km-video` is a workspace member, so its ffmpeg must be behind a feature

It is in `[workspace] members`, so `cargo build --workspace` compiles it whether or not anything wants
video. An unconditional ffmpeg dependency there makes ffmpeg and libclang a requirement of building
the **workspace at all**. That is not what this repository promises, and not what CI installs. It is
exactly the shape where an IDE that builds the whole project fails, while building one package is
fine.

So the crate carries one default-off feature, with a `cfg` at its root. A bare workspace build then
compiles it to an empty library in about a second. **Consumers do not name that feature.** The root
manifest puts it on the workspace dependency entry. The three crates that take `km-video` therefore
write `{ workspace = true, optional = true }`, and still get a real one.

### A `#[cfg]` block nothing compiles is a block that rots silently

A build without the feature does not typecheck the `#[cfg(feature = "video")]` block. A refactor that
converts every call site except the ones inside it therefore breaks `--features video` outright,
**while every ordinary command keeps passing**. They are all feature-explicit, and none of them turns
video on.

The only two things that compile that block are a workflow and a script somebody has to remember to
run. **Before a change to any of the four crates that touch video, `cargo km-lint` costs one command
and is the whole of the protection.**

### `--all-features` is wrong on Linux — never use it on this workspace

`--all-features` does not mean *video*; it means **every** feature, and one of them is the package
builder's `desktop`, whose `wry` links libwebkit2gtk at load time. Linux deliberately never installs
that library, because a Linux build of that tool has no window. One carrying `wry` would not *start*
on a machine lacking it, and `--browser` could not rescue it. The failure is in the dynamic loader,
before `main`. **The one flag meaning "everything" asks Linux for the one thing Linux is designed not
to provide.**

Three things make that invisible:

- **It works on Windows and macOS**, where WebView2 and WKWebView need nothing at build time. **A flag
  that is wrong on one platform and right on the other two is not a flag anybody re-reads.**
- **The error names a `.pc` file.** It therefore reads like a machine that needs a package installed,
  rather than a feature that should never have been on. **The obvious remedy is the wrong one, and it
  is one `apt-get install` away from being taken.**
- **A second command running the identical flag is not a second line of defense.** Two copies of a
  wrong command are one fault.

**So the feature list lives in one file**, `tools/setup/features.sh`, sourced by everything that can
source a shell file. `.cargo/config.toml` spells it a second time, because a cargo alias cannot. So
**`check.sh` asserts the two agree**, and fails naming both if they do not. A copy nothing compares is
a copy that drifts. `ci.yml` runs the aliases, so it holds no copy of its own.

The assertion's pattern ends in a trailing space, and **that is load-bearing**. Without it a prefix
would also match the `-no-video` alias, and the two assertions would silently check the same line.

### The plain name is the video one

`cargo km-build`/`km-test`/`km-lint` are the video commands, and the `-no-video` twins are beside
them. `check.sh` runs the video list, and it is the script this project actually relies on.
`km-build-no-video` is a bare `cargo build --workspace`, and by the rule at the top of
`.cargo/config.toml` it needs no alias at all. It exists so `task build:no-video` forwards to one,
because **a task spelling a cargo line nothing compares is exactly the drift that file prevents.**

**Two things this is not.** It is not a change to the cargo `video` feature, which stays off by
default. A fresh clone still compiles the workspace with no C toolchain. It is not a change to CI
either, whose runners have neither ffmpeg nor libclang. And it stops short of the per-tool aliases,
because `km-pack book` and `km-pack check` open no song bytes and want no ffmpeg.

### Where the dependency `COPY` sits in the Dockerfile

Docker's layer cache is keyed on the parent layer's ID plus the instruction string — plus, for a
`COPY`, the checksum of what is copied. **A volatile file belongs low and a stable one high.** The
dependency script sits *last*, below rustup and `cargo install cargo-deb`, so editing the dependency
list invalidates nothing above it. The few packages the portable tarball needs sit *up* in the first
apt layer, where they are protected. Put the dependency script above rustup, and editing it rebuilds
rustup, the `cargo install` and everything after. That is several minutes, for a change that has
nothing to do with either.

**This is not defeated by the image tag hashing both files.** A new tag is a new name, not a cold
cache — every layer above the change still matches on instruction and parent. The tag changing is what
stops two checkouts clobbering each other; the ordering is what stops an unrelated rebuild. **Different
jobs, and it is easy to assume the first cancels the second.**

## Android prerequisites

Both are large downloads and neither can be installed from inside the project.

| Needed | Notes |
|---|---|
| Android SDK + NDK + `cmdline-tools` | under the platform's SDK location |
| **JDK for Gradle** | **17 or 21.** Gradle 8.12 will not run on 25 — `Unsupported class file major version 69` |
| The JDK that is probably already there | **Android Studio's bundled JBR is 25**, too new. There is often no JDK on `PATH` and only an old JRE in the usual place, **so checking either of those suggests, wrongly, that no JDK exists at all** |
| `cargo-ndk`, the two Rust targets, `ninja` | the Ninja generator needs the last of these |

**A Gradle *toolchain* does not help with the JDK.** The failure is in the daemon JVM that compiles
the build script. A toolchain only governs what compiles the app.


---

# Command reference

Every command this repository has. The sections above are the narrative: prerequisites, a quick
start, what to do per platform. This is the complete list, with the traps that are otherwise learned
the hard way.

`task --list` prints the routine ones with a sentence each. **Every task forwards to a cargo alias or
a script under `tools/`, and each of those works with `task` absent.** The Taskfile is the index and
never the definition.

## Build, test, lint

```sh
cargo km-build          # the workspace, video included        (task build)
cargo km-test           # the test suite, video included       (task test)
cargo km-lint           # clippy over every target, -D warnings (task lint)
cargo fmt --all                                              # (task fmt)
task check              # all of them, in the order a failure is cheapest to read
```

**The plain name is the video build**, so these need ffmpeg's development libraries *and* libclang.
That is a deliberate default. Alternating between the two feature sets invalidates every crate
downstream of `km-video`, so the short name should be the one you actually want.

```sh
cargo km-build-no-video # = cargo build --workspace           (task build:no-video)
cargo km-test-no-video  # ...and the two `testing` features
cargo km-lint-no-video
```

**The cargo `video` feature is off by default**, which is what makes those work. `km-video` is a
workspace member, so `--workspace` compiles it either way. It keeps its ffmpeg dependency behind a
feature of its own, so a bare workspace build compiles it to an empty library.

- **Never `--all-features` on this workspace.** It turns on the two `desktop` features, whose `wry`
  links libwebkit2gtk, which Linux deliberately never installs. See `CONTRIBUTING.md`.
- **Do not type the feature list out.** It lives in `tools/setup/features.sh`. `.cargo/config.toml`
  spells it a second time, because a cargo alias cannot source a shell file.
  `tools/platform/linux/check.sh` asserts the two agree. A third copy is the drift that arrangement
  exists to prevent.
- **There is deliberately no Makefile.** `make` is absent from Windows by default, and cargo can do all
  of it.

```sh
tools/platform/linux/check.sh          # fmt + clippy + tests on Linux, in Docker (task check:linux)
tools/platform/linux/check.sh --quick  # skip the format check
tools/platform/linux/check.sh --shell  # a prompt in that image
```

It is worth running before a push. Three failures appear only off Windows: a `Path` treating `\` as
an ordinary character, a missing system library, and a wrong `#[cfg]`. All three have happened.

## Assets and toolchains — once per machine

```sh
tools/setup/fetch-assets.sh              # the GM SoundFont            (task assets)
tools/setup/fetch-assets.sh --list       # the banks it knows about
tools/setup/fetch-ffmpeg.sh              # ffmpeg + libclang           (task ffmpeg)
tools/setup/fetch-ffmpeg.sh --homebrew   # macOS: Homebrew's GPL build instead of a pinned LGPL one
tools/setup/fetch-abcmidi.sh             # for the carol pack          (task abcmidi)
task ffmpeg:android                      # the LGPL ffmpeg for both Android ABIs
```

**`fetch-ffmpeg.sh` writes both paths into cargo's own `[env]` table**, so neither variable has to be
exported. A build then works from any shell, and from an IDE that invokes cargo itself, which is the
case an `export` cannot reach. A real environment variable still wins. A project-local override file
is **not** an option: cargo's config `include` key is silently ignored on stable.

**An override bank is cached and never installed into `assets/soundfont/`.** Every carrier ships
everything under `assets/`, so a 206 MiB bank left there would go into every release.

```sh
task soundfont:list                    # the sixty-three banks, with what each measured
task soundfont BANK=musescore          # fetch it and play it here
task soundfont                         # ...which one is playing, and from where
task soundfont FILE=./banks/X.sf2      # anything already on disk
task soundfont:clear                   # back to the bundled bank
```

**This installs the bank into the machine's own SoundFont folder and writes its id into
`audio.soundfont`**, which every build on the box reads. It makes a hard link where the filesystem
allows one, so a 262 MiB bank is not duplicated per worktree. Across volumes it makes a copy.

The install is the part that matters. The folder is what says which banks exist, so a bank left only
in the shared download cache would be named and then not found. A *different* bank already there
under the same name is refused, naming both, rather than being given a `-2`. Two confusable rows in
the picker are worse than a refusal you can read. Nothing here touches `assets/` or `dist/`, so
nothing here can reach a release.

- **The bank is opened before anything is written**, and one that will not play is refused in the
  synthesizer's own words. Four of fifteen surveyed banks do not load at all. Naming one by hand
  gives a machine that comes up on a sine test tone, with the reason in a log.
- **The level travels with the bank.** Several exceed full scale at 1.0, so the volume is set with the
  path and the old value stashed beside settings.json. A level you edited by hand meanwhile is kept.
- **One bank cannot be fetched and never will be** — its terms forbid reproduction, so asking prints
  where to go and you finish with `FILE=`.

### Comparing two banks by ear

The commands above change which bank the machine *starts* on, which means a restart per comparison.
That is long enough that the first bank has left your ear when the second one plays. The switcher is
the other way. `Ctrl+1`…`Ctrl+9` change the bank mid-song, keeping the song and its position, so the
same bar can be heard under two banks a second apart.

```sh
task soundfont:debug:choose            # tick which banks go in the slots, from all sixty-three
task soundfont:debug:list              # the banks on this box, and the slot each would get
task soundfont:debug                   # fill Ctrl+2..Ctrl+9 with them
task soundfont:debug:clear             # empty the slots, turning the switcher off
```

- **`task soundfont:debug` can only reach the top of the table.** It keeps the first eight rows that
  are already cached, and the table is in rank order. The slots are therefore always the
  highest-ranked cached banks.
- **`:choose` is how you reach the other fifty-four.** It is a checkbox list of every bank, opened
  with the current slots ticked. It fetches whatever is ticked and missing, after telling you how
  much that is. Space ticks, typing filters, Esc changes nothing, and unticking everything empties
  the slots. A bank whose terms forbid mirroring is drawn but cannot be ticked, and saying so happens
  at the prompt. The list stays up, and nothing else you ticked is lost.
- **The list itself is `km-pick`**, a hundred lines around `inquire`'s `MultiSelect` that knows
  nothing about SoundFonts.
- **The slots are in table order either way.** The highest-ranked bank you tick is `Ctrl+2`, so the
  two commands cannot disagree about what a slot means.
- **`Ctrl+1` is always the bundled bank**, whatever is in the slots. That gives one digit that cannot
  be misconfigured, and a fixed reference to judge the rest against.
- **A non-empty list is the on-switch.** With no slots filled the keys do nothing, and nothing is
  drawn. With any filled, the name of the bank in force is on screen for the whole session. A
  recording of it therefore says which bank you were hearing.
- **Switching is run-only.** It never writes `audio.soundfont`, so whatever you press, the machine
  starts on the same bank as before and these commands cannot disagree with `task soundfont`.
- **It finds what has already been fetched**, from the asset cache — `task soundfont:list` says what
  there is and `tools/setup/fetch-assets.sh --bank <name>` gets one. `KM_SF2_DIRS` adds folders of
  your own, `;`-separated.
- **Every bank is opened before a slot is written**, and one that will not play refuses the lot.
  Nine banks written unchecked would be up to nine keys that each fail in front of a room.
- **A video or MP3+G song is not interrupted.** Those play through no synthesizer at all, so the
  bank is taken for the next MIDI song and the label says so.

## Running the machine

```sh
cargo run -p karaokemachine                       # the app  (or: cargo km, with video)
cargo run -p karaokemachine -- --headless         # no window: API + engine + catalog
cargo km -- --stream                              # no window either: the screen goes out as a stream
cargo run -p karaokemachine -- --fullscreen       # fill the screen, this run only
cargo run -p karaokemachine -- --windowed         # ...and the other way, against an install
cargo run -p karaokemachine -- --data-dir ./scratch
cargo run -p karaokemachine -- --api-bind 127.0.0.1:8277 --data-dir ./scratch
cargo run -p karaokemachine -- --port 8277 --data-dir ./scratch          # the same thing
cargo run -p karaokemachine -- --port 8277 --lan --data-dir ./scratch    # ...on every interface
```

**A build out of a checkout opens in a window, and an installed machine fills the screen.**
`display.fullscreen` defaults to off. A default reaches an install with no settings file, and it
reaches a checkout somebody has just built, so it cannot serve both. What tells them apart is who put
the machine there. The three setup programs therefore pre-write `{"display": {"fullscreen": true}}`
where there is no `settings.json`, beside the `first-run-soundfont.json` they already place.

`--fullscreen` and `--windowed` move one run either way and write nothing down; `F` still toggles.
See `The setup programs pre-write a settings file` in `docs/decisions/distribution.md`.

**`--stream` is the cheapest proof the screen draws on a platform.** It needs no display server, no X
and no Wayland. Start it, fetch `/stream/live.m3u8` and decode a frame. That exercises the fonts, the
CJK fallback, the layout, the wallpaper and every branch of `draw()`. It is why the stream runs over
SSH on a box with no monitor. What it cannot say is which backend SDL picks for a **window**, and
that wants a real session.

```sh
cargo km -- --stream --api-bind 127.0.0.1:8277 --data-dir ./scratch
curl -s http://127.0.0.1:8277/stream/live.m3u8          # the playlist, and the interface
```

**It is `cargo km` and not the bare `cargo run` its neighbours use.** `km-stream` rides on the
`video` feature, so a build without it stops at startup naming the encoder it has not got. The
segments are written under `--data-dir`, which is the other reason to give one. The icon in the bar
is the `tray` feature, which only the Windows and macOS staging scripts name. A run from a checkout
therefore puts none up.

**There are two binaries on Windows and `default-run` names the first.** `karaokemachine` is
GUI-subsystem, so double-clicking the staged exe opens the machine with no console beside it;
`karaokemachine-console` is the same library with a console. Ask for it by name:

```sh
cargo run -p karaokemachine --bin karaokemachine-console -- --help
```

- **`--api-bind` is for this run only** and is never written to settings.json. A bare port keeps the
  interface `api.bind` names; a full address says both.
- **`--port` and `--lan` are the same setting under the other three tools' spelling.**
  `km-package-builder`, `km-remote` and `km-admin` all take that pair, and so does `km-api`'s own
  `dev_server`; the machine took neither and was the odd one out. `--port 8277` is exactly
  `--api-bind 8277`, and giving both is refused rather than resolved. `--lan` needs `--port`,
  because `--api-bind` already says the interface. Note the machine's *default* is the opposite of
  the three tools'. `api.bind` ships as `0.0.0.0`, because a machine no phone in the room can reach
  is not one.
- **Pair it with `--data-dir`**, or two runs are on two ports and still share one catalog, one
  packages folder and one settings file.
- **Prefer the full loopback form on Windows**: a listener on `0.0.0.0` raises a firewall prompt per
  program and port, and a loopback one raises none.
- **The TCP listener is not the only socket that prompts.** An mDNS daemon binds UDP `0.0.0.0:5353`,
  so a program bound to loopback still raises the dialog if it browses for machines. `KM_NO_MDNS`,
  which `.cargo/config.toml` sets for every `cargo` call, is what closes that one.

```sh
cargo run -p karaokemachine -- --set-password hunter2      # change it; --reset-password goes back to a PIN
cargo run -p karaokemachine -- --set-soundfont ./X.sf2 [--music-volume 0.8]
cargo run -p karaokemachine -- --clear-soundfont
cargo run -p karaokemachine -- --first-run-soundfont recommended   # the next start fetches it
cargo run -p karaokemachine -- --set-debug-soundfonts ./A.sf2 './B.sf2=Bee=0.8'
cargo run -p karaokemachine -- --clear-debug-soundfonts    # `task soundfont:debug` is these two
cargo run -p karaokemachine -- --set-debug-packages ./vol1.kmpkg ./vol2.kmpkg
cargo run -p karaokemachine -- --clear-debug-packages      # takes no file off the disk
cargo run -p karaokemachine -- --show-debug-packages       # one bare path a line, or nothing
cargo run -p karaokemachine -- --show-paths                # settings, catalog, packages, assets,
                                                           # which bank won, and the overlay if any
cargo run -p karaokemachine -- --list-audio-devices
cargo run -p karaokemachine -- --frame-stats               # fps/draw/present/interval + decode, once a second
cargo run -p karaokemachine -- --log-file                  # ...and into a logs folder
cargo run -p karaokemachine -- --log-keep all              # ...and never delete one of them
cargo run -p karaokemachine -- -v                          # this crate's debug stream; -vv for all
```

- **`--music-volume` takes no meaning on its own** and is refused alone: it is half of one decision,
  not a volume control.
- **`--first-run-soundfont` writes a request and fetches nothing.** It takes `recommended` or a bank
  id. The *next* start downloads it, checks it plays and chooses it, and three starts try before it
  gives up. It is what the Windows and macOS setup programs' tick box writes. It is also the way to
  get the same behavior on the appliance, which has no tick box. `--show-paths` names a pending
  request; deleting that file, or choosing any bank yourself, calls it off.
- **`--set-debug-soundfonts` takes `<path>`, `<path>=<name>` or `<path>=<name>=<level>`**, in slot
  order from `Ctrl+2`. The name is what the on-screen label shows; the level is what that bank was
  measured to want. Reach for `task soundfont:debug` instead unless you are naming banks the table
  does not know.
- **`debug.wallpapers` is the same idea for pictures**, and has no flag. It is a list of extra images
  or `.zip` packs in `settings.json`, shown **on top of** whichever wallpaper folder the rules chose.
  `--show-paths` names the entries on `debug wall` lines and says which are missing. Editing the
  list and pressing the wallpaper-advance key picks it up with no restart. The folder choice and the
  extras are both re-read at every rescan. A path that is not there simply never appears —
  quietly, as an unreadable folder does, because this is a `debug.` list.
- **`--set-debug-packages` is how you install a `.kmpkg` that is not in a folder the machine
  scans.** It is *added to* those folders rather than replacing them. Every package is opened before
  anything is written, and each path is made absolute.
- **It is the successor to `settings.packages`, and it lives behind `debug.` because of how it
  fails.** Each entry is replayed at every pass. A file moved or tidied away is therefore a fault
  reported at every pass, until somebody clears it. **For a library that lives somewhere else, use
  `settings.package_dirs`** — naming a folder cannot rot the way naming a file does.
  `--clear-debug-packages` empties the list and **deletes nothing**; anything that was also in a
  scanned folder stays installed. `--show-paths` names the entries on `debug pkg` lines, and says
  which of them are missing.
- **`--list-audio-devices` prints one line per physical output first, then the other names for those
  same outputs below a paragraph.** The first block is the operating system's own device list; the
  second is alsa-lib's configuration over it. Any id in the first block is the one worth saving. It is
  read-only — choose with the API or `audio.output_device`.
- **The machine is quiet by default**, plain `info`. `--frame-stats` is the only thing that builds
  the meter, and `KM_FRAME_STATS=1` too, for a systemd unit. `RUST_LOG` overrides the `-v` ladder and
  does *not* bring the meter back, and neither does `logging.level`.
- **On a retail Android TV `--frame-stats` cannot be reached, but `F12` can — so the meter is
  available there after all.** The flag is genuinely unreachable. `SDL_main` never sees an argument
  vector, so `KM_FRAME_STATS=1` is the only route. Setting an environment variable for an app needs
  the `wrap.<package>` system property, and a retail build's SELinux policy refuses `adb shell` that
  outright. It fails as *"Failed to set property … See dmesg for error reason"*. A short package
  name fails as readily as a long one, so it is the policy and not the 32-character name limit.

  **What closed the gap is that the meter stopped depending on the flag.** `F12` builds it too, and
  `input::action_for` maps that key with no platform gate — so `adb shell input keyevent
  KEYCODE_F12` draws the panel on a retail television. Verified on a Google TV Streamer. So
  measuring frame timing there needs no throwaway build with the flag forced on. The panel is the
  *better* instrument anyway, because it puts the numbers where the person who can see the stutter
  is sitting.

  The remaining limitation is narrow and worth stating. `F12` gives the panel and **not** the log
  line, since the flag is what decides whether the meter also writes one. A television can be read
  from the sofa and still cannot be made to journal.
- **A video song that could not be decoded in real time warns without any of that**, which is what
  makes the gap survivable. `starved_ms` is how long the sound ran dry, which is how long the song
  visibly and audibly stopped. It is reported at `warn` when the song ends, on a default `info`
  machine with no flag set. It is a fault rather than a measurement. A healthy song reports nothing.
  The per-second version rides the `--frame-stats` line.
- **`--log-file` is for the runs with no console**, which is what a double-clicked GUI-subsystem exe
  is. Its standard output handle is null, and every line is *discarded*. One file per run, ten kept.
  `KM_LOG_FILE=1` does the same where there is no command line. It does **not** ride the `-v` ladder,
  so `-v --log-file` is how you get a detailed one.
- **`--log-keep <count|all>` says how many to keep, and turns the file on by itself.** `all` is for a
  machine being worked on. The names carry the date and time each run started. A folder that is never
  pruned is therefore that machine's whole history, in the order it happened. `KM_LOG_KEEP` does the
  same with no command line, and the package builder, the offline remote and `km-admin` read it too.
- **`settings.json` says all of it permanently**, for the programs nobody types at. Four keys in one
  `logging` section, and the flag wins over the variable, the variable over the file:
  ```json
  "logging": { "level": "info,km_api=debug", "file": true, "keep": "all", "ecapplog": true }
  ```
  **Three programs read that section, and it is the same section in all three.** They are the
  machine's `settings.json`, the package builder's in its config folder, and `km-admin`'s in its data
  folder. The offline remote keeps no settings file, so its flag and `KM_LOG_FILE` are the whole of
  what it reads.

  `level` takes what `RUST_LOG` takes, from a bare `"debug"` to a list naming targets. It replaces
  the `-v` ladder rather than moving along it, which is how it says the thing a rung cannot. `keep`
  takes `"all"` or a number. `ecapplog` takes `true` or an address. **A value that will not parse
  leaves the built-in behaviour standing, and says so in the log.** It does not stop the program, and
  it is not guessed at.

  The section is **not** under `debug`, deliberately. `debug.enabled` publishes a passwordless copy
  of the API, and asking for a log must not be a way to arrive at that.
- **`--ecapplog` puts the log in the [ECAppLog](https://github.com/RangelReale/ecapplog) viewer
  instead of the console**, with a tab per crate and a level to filter on. The machine, the package
  builder, the offline remote and `km-admin` all take it. The viewer need not be open first — lines
  wait for it — and `--ecapplog=192.168.1.x:13991` reaches one on another computer. `--log-file` and
  the machine's own log page are unaffected, and the `-v` ladder still decides what goes in it.
- **`KM_ECAPPLOG=1` turns that on for every program at once**, which is what a checkout wants while
  this is being worked on. Put it in `.cargo/config.toml`'s `[env]` block, beside `KM_NO_MDNS`.
  Nothing an owner installs reads it, a release never running through cargo. An address works there
  too, and `KM_ECAPPLOG=0` in front of one command turns it off for that run. `logging.ecapplog` is
  the third rung, in the `settings.json` of each of the three programs that keeps one.
- **A panic writes its own file whatever those two say.** `<stem>-<UTC>.crash`, in the same folder,
  holding where it happened, what it said, the thread, the version and a backtrace. Nothing is
  written unless one happens. Crash reports are counted apart from run logs, so an evening of
  restarts cannot delete the report of the panic that ended one of them.

```sh
cargo run -p karaokemachine -- --play fixtures/generated/soft_karaoke_header_on_words_track.kar
cargo run -p karaokemachine --features video -- --play ./clip.mp4
cargo run -p karaokemachine -- --play ./song.cdg      # give either half; the other is found beside it
```

**The song has to be written before it can be played**, because **this repository commits no song at
all**. Every karaoke file the tests and examples use is bytes written by hand.

```sh
cargo run -p km-song --features testing --example write_fixtures   # once: fixtures/generated/
```

It writes a second set too: the files that must be *refused*. Point a tool at those when you want to
see what it says about a bad file.

## The song book

```sh
cargo run -p karaokemachine -- --song-book ./songbook.pdf [--book-name "Sala de Estar"]
```

This is the paper half of a karaoke machine. It has four columns, alphabetical by artist inside a
section per language, modeled on the book a commercial machine ships in a ring binder.

- **It reads the catalog as it stands.** Packages are indexed when the machine *starts*, so a
  `.kmpkg` dropped in since the last start is not in the book. The symptom is indistinguishable
  from a package that failed to install. The command says so on every run.
- **No font ships with it, and that is the feature.** A PDF reader supplies the glyphs for the base-14
  fonts, which is what lets the crate take no external dependency at all. The price is cp1252:
  non-Latin characters become `?` and are **counted**, with the count printed.
- **The numbers are the ones this machine assigned.** `km-pack book` prints the bank each package
  *suggests* instead; they usually agree.
- **Two strings at the top, not one.** `--book-name` is whose machine the book is for; the centered
  title is a separate thing only `km-pack book --title` sets.

`GET /api/v1/songs/book.pdf` is the same document over HTTP, taking `?language=`, `?package=` and
`?name=`. **`?name=` is hashed into the `ETag`, never interpolated.** It is the only free text that
could reach a header, and the body varies with it.

## Packaging

```sh
cargo km-pack spec ./songs --out vol1.kmspec.yaml   # describe a folder
cargo km-pack build vol1.kmspec.yaml                # build what it describes
cargo km-pack check vol1.kmpkg                      # validate + report
cargo km-pack check vol1.kmpkg --verify             # + every entry against its checksum
cargo km-pack book vol1.kmpkg vol2.kmpkg --out brasil.pdf
```

**`--verify` reads the whole package**, which for a video library is most of a minute, so it is off
by default. It answers *did this arrive intact?* where the rest of `check` answers *does this
describe itself properly?* — reach for it on a package that came over a network or off a stick.

**`build` takes a description and nothing else — never a folder.** `spec` walks a folder and writes
down every decision a build would otherwise make silently. Correcting a title there is all it takes,
because the build compares what you wrote against what the file says. It records the difference as a
correction. **There is no `edited:` key to maintain, deliberately**: a stored list of corrections can
lie about itself and a comparison cannot.

The selection flags live on `spec`, because selecting is what a description records: `--min-suitability`,
`--require-lyrics`, `--limit`, `--index`, and `--from OLD.kmpkg` to seed hand-edited titles.

```sh
cargo km-pack spec ./songs --out vol1.kmspec.yaml --default-language und
cargo km-pack-video build vol1.kmspec.yaml
cargo km-pack-video spec ./songs --out v.yaml --no-transcode
```

- **A build refuses a song with no language.** Most MIDI files classify themselves; what is left is
  video songs and plain `.mid` files with no header. `und` is the standard's own "undetermined" and is
  the honest answer.
- **The same command packages MP3+G pairs, with no feature and no ffmpeg.** Nothing is ever
  transcoded, because every intuitive quality signal was tried against 2,849 real files and every one
  was wrong. **Orphans are reported, never skipped silently.**
- **UltraStar songs are packaged from their `.txt`, with no feature and no ffmpeg.** The header names
  the MP3, and the package holds that MP3 and the lyric timeline read from the file. A text file that
  is not an UltraStar file is not listed and not reported.
- **Videos are checked against one profile and copied byte-for-byte when they already match**, which a
  download normally does. `--no-transcode` stores irregular files as they are; one the machine could
  not *play* is still refused.
- **A description `spec` writes says `uncurated: true`**, and the build marks the package with it.
  Nobody has reviewed a walk of a folder. Delete the line once somebody has. `inspect` and
  `check` print a package's flags by name, and the listing says it too. See
  [`An uncurated package says so everywhere but the television`](docs/decisions/packaging.md#an-uncurated-package-says-so-everywhere-but-the-television).
- **`book` needs no `video` feature and no ffmpeg**, unlike every other command that meets a video
  song. A book is manifest metadata and never opens a song's bytes.

### Levelling: measuring how loud a media song is

A build measures every video and MP3+G song's loudness, and writes it into the manifest. The machine
can then bring it down to the level its bank renders MIDI at — see
[`Video and MP3+G play at the MIDI reference level`](docs/decisions/audio.md#video-and-mp3g-play-at-the-midi-reference-level).

**A MIDI song is not measured here and needs nothing from a build.** The machine reads its level out
of the events as it starts, in a fifth of a millisecond. It moves the level to the same place, up as
well as down, which is what answers a file that plays too low. So a package built before any of this,
and a file played straight from disk, are both levelled with no rebuild. `audio.normalize_midi: false`
in `settings.json` turns it off, separately from `normalize_media`. See
[`Every song plays at the level its bank renders the corpus at`](docs/decisions/audio.md#every-song-plays-at-the-level-its-bank-renders-the-corpus-at).

```sh
# What a folder of MIDI files renders at, and how close the estimate is to it. Needs a bank.
KM_CORPUS=<your karaoke folder> cargo run --release -p km-audio --example loudness_census -- \
    "$KM_CORPUS" --limit 1000 --bank <bank.sf2> --tsv rows.tsv
# ...and re-check the estimate against those rows after changing it, rendering nothing again.
cargo run --release -p km-audio --example loudness_census -- --refit rows.tsv
```

`--refit` prints the sample's mean beside `km_loudness::MIDI_REFERENCE_ESTIMATE`, which is the one
constant a change to `km_song::loudness` has to move.

```sh
cargo km-pack inspect vol1.kmpkg --songs           # the LUFS and the gain, per song
cargo km-pack-video build v.kmspec.yaml --no-loudness   # skip measuring
# Measure a package already built. `--release` in full, for the reason below.
cargo run --release -p km-pack --features video -- reanalyze old.kmpkg --out new.kmpkg
```

- **It costs a full audio decode per media song, about 0.6 s each, and that is a release figure.** A
  video's picture is never decoded. `--no-loudness` is for somebody iterating on a description who
  does not want to pay it each time. A song built without it plays unlevelled.
- **Measure in release, and `cargo km-pack` is not.** That alias and `km-pack-video` are debug builds,
  where symphonia decodes roughly thirty times slower. A 223-song MP3+G package took **70 minutes of
  CPU and printed nothing** before it was killed, against about two minutes optimized. Nothing warns,
  because a decode that is thirty times slower is still a decode. So a measuring run is spelled in
  full — the same reason `km-wallpaper-pack` carries `--release` in its own alias, whose `analyze`
  took 78 minutes unoptimized. The other subcommands here are cheap and the plain alias suits them.
- **`reanalyze` is the migration path, and it needs no sources.** It measures the media *inside* an
  existing package and rewrites only the manifest. Nothing is re-encoded, and the result comes out
  the same size as a freshly built package carrying the same numbers. It also re-derives MIDI
  suitability, so its `--no-loudness` skips only the measuring.
- **A video needs the `video` feature to be measured**, so use the `-video` twins above. Without it a
  video is carried across unmeasured and says so; an MP3+G song is measured in any build, because
  `km-cdg` is pure Rust.
- **`inspect --songs` prints the measurement and the gain beside it** — `-5.5Lu x0.15` is a
  commercially mastered karaoke MP3 coming down 16 dB. The gain shown is against the default
  reference; the machine works the real one out from the bank that is sounding.
- **`audio.normalize_media: false` in `settings.json` turns the whole thing off** on a machine,
  without un-measuring anything.

Drop the `.kmpkg` into the packages folder `--show-paths` names and restart, or install it without a
restart with `POST /api/v1/admin/packages`.

## The simple packager

```sh
cargo km-package-simple                         # the first page asks for a folder
cargo km-package-simple ./songs                 # ...or read this one at once
cargo km-package-simple-video ./songs           # a folder holding video songs
cargo km-package-simple-desktop                 # ...in a window
```

It serves `http://127.0.0.1:8181/`, or any free port when that one is taken. It keeps a settings file
with the language and the last folder, and nothing else. Every package it writes carries the
`uncurated` flag, and a listing goes beside each one. See
[`A package can be built straight from a folder`](docs/decisions/curation.md#a-package-can-be-built-straight-from-a-folder).

## The curation tool

```sh
cargo km-package-builder                        # reopen last, or the Open page
cargo km-package-builder --pick                 # the list, not last time's folder
cargo km-package-builder ./songs --init --scan --open
cargo km-package-builder ./songs --machine http://127.0.0.1:8177
cargo km-package-builder --register             # double-clicking a .kmbuild opens it
cargo km-package-builder --unregister
cargo km-package-builder ./songs --backup kept.json    # write and exit
cargo km-package-builder ./songs --restore kept.json   # read and exit
cargo km-package-builder ./songs --restore kept.json --overwrite
cargo km-pkgbuild-desktop                       # ...in a window
cargo km-pkgbuild-desktop --browser
```

**`--init` is the only thing on the command line that creates a database**, so a wrong folder is an
error rather than an empty index. The Open page's *Create here* is the other.

- **Its database is the document you double-click**: `km-package-builder.kmbuild`, ordinary SQLite.
  Opening looks for whatever single `*.kmbuild` a folder holds, so a corpus can be `Brasil.kmbuild`.
  **Nothing silently falls back to an older name.** An older file is reported with the rename spelled
  out, and the Open page offers to do it.
- **`--register` is per-user and needs no elevation.** It records where the exe is *now*, so re-run it
  if the folder moves. On macOS the bundle declares the type, and the flag only nudges
  LaunchServices.
- **The `desktop` feature is never on for Linux**, because `wry` links libwebkit2gtk at load time.
  `--browser` declines the window anywhere, and **a window satisfies `--open`**.
- **On Windows with that feature there are two executables**; the `-console` one is what answers
  `--help`.
- **A build runs on its own thread and shows a bar**, locking briefly at each end and not at all
  across the work. Closing the folder mid-build waits for the song in hand, not the run.
- **The Settings page's Discover button lists machines and never sets one.** This tool installs
  packages, so one that re-pointed itself at whatever answered first would eventually write to the
  wrong machine.
- **`--backup` writes only what a person typed.** That is the half a re-scan cannot rebuild: titles,
  artists, languages, encodings, transpositions, the hand-set score, notes, merges and favorites.
  Not packages and not duplicate verdicts — a built package re-opens with the Packages page's
  *Import*. The Settings page has the same two buttons, and says how many songs carry anything.
  **Keep the file off the drive the corpus is on.**
- **The flag takes a path verbatim; the Settings page's default carries the moment.**
  `km-package-builder-20260909T140233Z.kmbackup.json` goes in `_kmbuild-data`, so a second backup
  sits beside the first rather than replacing it. The restore box suggests the newest one it finds.
- **`--restore` rejoins by the hash of a song's bytes, so scan first.** A song this folder has not
  indexed is listed rather than invented. It fills only what is blank unless `--overwrite` is passed,
  and **never blanks a field the file says nothing about** — under either setting. A value it will
  not take is a line in the report rather than a failed run. Both flags refuse `--init`: there is
  nothing to rejoin to in a database that does not exist yet.
- **`KM_PACKAGE_BUILDER_RECENT` names this run's recent-folder list**, and set-but-empty keeps none —
  nothing remembered and no startup reopen. Every other run writes `recent.json` in the per-user
  config folder. That list holds twelve entries of somebody's own curation, and it is never pruned of
  folders that have gone away. The startup reopen takes the first of them. **A screenshot run, a
  smoke test or a scripted build sets this**, so that naming a throwaway folder does not evict a real
  corpus. There is no flag, because the runs that must not write here are the ones a script starts.
- **`KM_PACKAGE_BUILDER_PASSWORDS` names this run's remembered machine passwords**, with the same
  three states. Every other run writes `machine-passwords.json` beside the list above. That file maps
  a machine id to the admin password somebody ticked "remember" for, `0600` where the platform has
  it. It is deleted rather than emptied once the last one is forgotten. **A run that talks to a
  machine sets this too**, for a stronger version of the reason above. A smoke test that wrote here
  would be putting a credential into a store its owner believes they control.

## KaraokeMachine Admin

Finds pictures and SoundFont banks for a machine, and sends those — and files you already have — to
it. **In the second workspace**, `tools/cmd/assets` — see `The wallpaper pack` below for why that
workspace exists. The aliases carry its `--manifest-path`, so nothing here has to spell it.

```sh
cargo km-admin                             # http://127.0.0.1:8180
cargo km-admin --machine 192.168.1.5       # when discovery cannot find one
cargo km-admin --data-dir ./local/km-admin   # downloads, packs and settings
cargo km-admin --port 8480 --lan           # a second one, reachable from the house
cargo km-admin --open                      # ...and open a browser at it
cargo km-admin --log-file                  # ...and into a logs folder; -v / -vv say more
cargo km-admin-desktop                     # ...in a window
cargo km-admin-desktop --browser
```

- **It is the other half of `/admin/`, and it *serves* `/admin/` too.** The machine takes files and
  cannot go and find one. It may have no internet, and it should not hold somebody's Pixabay key. It
  also has better things to do than a hundred JPEG decodes. This is what produces the file — and the
  page around that is the machine's own, from `km-admin-pages`, rather than a copy of it.
- **Everything is under `/admin`, and `/` redirects to the front door.** That is where the shared
  markup's links point. This program's own pages are `/admin/connect`, `/admin/pictures/find` and
  `/admin/sound/fetch`. A link from the shared Pictures and Sound tabs reaches the last two. The
  searching is a page under its tab rather than the tab itself. That is a consequence of sharing the
  markup, and `docs/decisions/distribution.md` argues it.
- **The redirect is temporary and a shell opens the door itself.** A browser keeps a permanent
  redirect and follows it without asking again. That would make where the door is a promise every
  later build has to honour.
- **It opens on the front door, and that is where a machine is chosen.** The page has no tab strip.
  It shows the machine it last chose, what is advertising itself over mDNS, a box for an address and
  a box for the password. It opens on every launch, and the tabs are not drawn until a machine is
  picked. Whichever row you press is remembered in `<data-dir>/machine.json`. `--machine` is
  therefore for a machine broadcast cannot reach, or for pointing one run somewhere else without
  disturbing what is remembered.
- **Nothing is set until you press a row**, the rule `km-package-builder` keeps and for its reason.
  This program uploads files, and re-pointing itself at whatever answered first would eventually put
  one on the wrong machine. Two things look like faults and are not. A machine serving on
  `127.0.0.1` advertises nothing at all, so a loopback test instance never appears. A machine also
  appears only while it is running, and only with `api.advertise_mdns` on.
- **The pictures come through `km-wallpaper-pack`'s own three phases**, contrast gate and all, driven
  from a page rather than a command line. Openverse needs no key; Pixabay and Pexels need one of
  **your own**, and a pack built from either is for the machine that built it.
- **The banks come from the same table the machine compiles in**, `km-banks`, verified against the
  digest it pins. A machine with its own internet connection can fetch these itself. This is for the
  one that cannot, and for the television box with no shell.
- **The Songs tab sends a `.kmpkg` you already have**, and Pictures and Sound each take one too.
  Nothing here searches for songs — a package is yours and no stock library has one. It is for the
  television box, which has no shell and no file manager that reaches where the machine looks. The
  extension and the size are checked against the machine's own `km_api::uploads` table; everything
  else is the machine's answer in its own words.
- **Everything it *makes* goes to `--data-dir` as well as to the machine.** A machine that is
  switched off is therefore a delay rather than a dead end. A file you already have is staged under
  `<data-dir>/staging` for the length of the transfer, and removed afterwards. No copy is kept, and
  the folder is emptied at startup for whatever a kill left behind.
- **`--lan` is off by default and rarely right.** This program has no password of its own, holds whatever API
  keys it has been given, and writes files as you. It says so when you ask for it.
- **Its mark is the fourth palette**, magenta, and it has a favicon and a tray icon like the other
  three. The tray does not depend on the window: the run that declined one is the run with nothing
  else to show for itself.

## The offline remote

```sh
cargo km-remote                             # http://127.0.0.1:8179
cargo km-remote --machine 192.168.1.5       # when discovery cannot find one
cargo km-remote --refresh                   # re-read the catalog regardless
cargo km-remote --port 8379 --lan           # a second one, reachable from the house
cargo km-remote --data-dir ./scratch        # keep a run out of the real favorites
cargo km-remote --open                      # ...and open a browser at it
cargo km-remote --log-file                  # ...and into a logs folder; -v / -vv say more
cargo km-remote-desktop                     # ...in a window, icon in the bar
cargo km-remote-desktop --browser
```

Holds **this device's own copy** of a machine's catalog, so browsing, searching and favorites work
with the machine switched off.

- **Two databases, deliberately separate.** The catalog mirror may be thrown away and rebuilt. The
  favorites are a collection somebody built up over a year, and must not go with it.
- **A typed address pins the machine**, which is what `--machine` means, and **rescanning is the way
  to take the pin off**. Nothing is remembered until it has actually answered.
- **`--lan` is off by default**, which is *not* the choice the machine's own remote makes. That one
  is open to the house, because anybody in the room should be able to queue a song. This one holds
  one person's favorites.
- **Queueing works against any machine.** Everything the offline remote drives is public, so a
  machine having a password costs it nothing. It still cannot reach anything under
  `/api/v1/admin/`, and has nowhere to type a password — but nothing it offers is there.

## Diagnostics and corpus tools

```sh
cargo km-lyrics dump song.kar                            # parsed timeline + analysis
cargo run --release -p km-lyrics -- scan /path/to/corpus             # formats, encodings, line widths
cargo run --release -p km-lyrics -- scan /path/to/corpus --as-written # the same, before a run is re-broken
cargo run --release -p km-lyrics -- preview /path/to/corpus --limit 30000
cargo run --release -p km-cdg --example stills -- song.cdg [--at 12,45,90 --scale 4]
cargo run --release -p km-cdg --example scan -- /path/to/corpus
cargo run -p km-audio --example render_wav -- in.kar out.wav       # offline, no audio device
cargo run -p km-audio --example render_wav -- in.kar out.wav bank.sf2 --only-channel 8 --dump-channels
cargo run --release -p km-audio --example bank_info -- bank.sf2    # what a bank holds, and what loading it lost
cargo run --release -p km-song --example event_census -- /path/to/corpus [limit]
cargo run --release -p km-fixes --example fix_census -- /path/to/corpus [limit] [stride]
KM_CORPUS=<your karaoke folder> tools/dev/soundfont-measure.sh     # the research note's §3 and §4, re-run
# What a page and a scan batch cost on a corpus-sized .kmbuild. Ignored tests, because they need a
# real corpus and the page cache emptied first -- see `docs/research/sqlite-mmap.md` for the regime
# and `db::measure`'s own header for why one run measures one setting.
KM_CORPUS=<a folder holding one .kmbuild> KM_MMAP=on cargo km-test --release -- \
    --ignored --exact db::measure::a_cold_page_load_over_a_real_corpus --nocapture
KM_CORPUS=<...> KM_MMAP=off KM_SAMPLE=4000 cargo km-test --release -- \
    --ignored --exact db::measure::a_bounded_forced_pass_over_a_real_corpus --nocapture
# How many files a scan should read at once, on the disk the corpus lives on. One arm per value in
# KM_JOBS, each over files no other arm read -- so this is the one that needs no emptied page cache.
# Give the list twice in opposite order: a value whose two arms disagree measured the warming.
KM_CORPUS=<...> KM_MMAP=on KM_SAMPLE=4000 KM_JOBS=24,1,16,2,8,4,4,8,2,16,1,24 \
    cargo km-test --release -- \
    --ignored --exact db::measure::how_many_readers_a_disk_wants --nocapture
# Where the same-words threshold sits: what two files of one recording score against what a
# coincidence scores, how often the phrases reach the other file, and what one search costs.
KM_CORPUS=<...> KM_MMAP=on cargo km-test --release -- \
    --ignored --exact db::measure::where_the_same_words_threshold_sits --nocapture
cargo run -p km-songbook --example sample                          # a synthetic book, to judge by eye
cargo km-preview                          # every screen to target/preview
cargo km-wallpapers                       # the gradients (NOT the default set)
cargo km-icon                             # every app icon, every platform
cargo km-banner                           # the Android TV banner
cargo run -p km-api --example dev_server --features testing        # the API over an in-memory machine
BASE=http://127.0.0.1:8177 tools/dev/remote/api-walkthrough.sh     # every endpoint, with curl
```

**Use `--release` for the corpus scans.** Warm, the CD+G one replays 46 million packets a second, so a
2,849-file corpus takes 4.5 s where a debug build takes minutes.

**A person judges the picture examples by eye**, like the screens. A palette read wrong, a tile
bitmap reversed or a baseline off by a row all produce output that still *looks* like output.

**`render_wav`'s two diagnostic flags are for one part sounding wrong in a mix that does not.**
`--only-channel` silences every other channel, which is how a part is judged at all. `--dump-channels`
reports the pitch bend range and tune each channel ended on. It answers the question a WAV cannot:
whether the file's RPN setup was acted on or discarded.

A part written as bends against a twelve-semitone range, and played at the default two, is out of
tune on every note. The fraction differs from note to note. The dump says `12.00`, or says nothing.

**The three SoundFont tools answer three different questions, and only the last one is slow.**
`bank_info` prints one file's contents and what the synthesizer dropped to load it. Those are the
columns [`soundfont-banks.conf`](crates/machine/km-banks/data/soundfont-banks.conf) carries.

`event_census` answers "how rare is that MIDI message, really". Leaving that shape of question
unmeasured once produced a wrong comment about aftertouch.

`fix_census` beside it answers the same shape of question about a *defect*: how many files carry one
this machine can correct. It takes a **`stride`**, because a bare limit reads the first files the
walk reaches. A walk reaches them a folder at a time. On a corpus whose folders came from different
sources, that is a figure about whichever few happened to be first. Every twenty-fourth file spreads
the same number of reads across the whole of it.

It parses rather than searching the bytes, which matters for exactly this question, and in both
directions. `B4 00 7F` occurs inside track names and delta times, where it is not a bank select. A
bank select written under running status carries no `B4` for a search to find. Over the corpus the
second outweighs the first, so the byte search reads low.

`soundfont-measure.sh` re-derives that table's spread, volume and **lufs** columns over every bank in
the asset cache, seven songs each. It **never downloads**. A bank that is not cached is named and
skipped, since several of the pinned banks come from a slow archive host. It needs `KM_CORPUS`,
because the seven songs live in a local folder that no committed file may name.

**`lufs` is the one column of the three the machine acts on.** It is the reference a video or MP3+G
song is attenuated to. A bank added to the table without one falls back to a constant, and is
levelled slightly wrong. Its `Mean LUFS` output is that number.

**Re-run it after a `rustysynth` change and record the commit.** The branch can advance twice in an
afternoon. A run is comparable with the recorded rows when it reproduces their `spread` to the
decimal.

## The Christmas carol pack

```sh
tools/dist/carols.sh              # fetch, convert, build, report   (task carols)
tools/dist/carols.sh --keep-work  # ...keeping the generated ABC and .kar files
```

Sixteen public-domain carols as one `.kmpkg`, **a separate download and never bundled**. Needs the
network once and `abc2midi`. The license gate is in code and **fails closed**. Each carol's own
copyright line must say public domain in all four layers a hymn divides into.

## The landing page

```sh
tools/dist/site.sh                # stage dist/site                   (task site)
tools/dist/site.sh --open         # ...and open it                    (task site OPEN=1)
tools/dist/site.sh -v             # ...naming every file it staged
```

One hand-written page, `site/index.html` and `site/style.css`, staged with the eight screenshots out
of `docs/images/` and a favicon out of `icon/`. **`.github/workflows/pages.yml` runs this exact
script** and uploads what it produces, so a local preview and the published page come out of one
code path. It publishes to <https://rrgmc.github.io/karaokemachine/>, and only once the repository
is public — the workflow tests the repository's name *and* its visibility and skips otherwise.

**Opening `site/index.html` from the checkout shows no pictures**, and that is by design. The paths
are relative to the *staged* folder, which is the only place they resolve. Staging is the preview.

It refuses three things that would otherwise be found only after publishing:

- a picture the page names that `docs/images/` does not have;
- an absolute path, the site being served under `/karaokemachine/`, so `/images/x.png` would 404;
- anything the page would fetch from another server.

## Issue labels

```sh
tools/dev/labels.sh list                        # the table, for a person   (task lint:labels checks it)
tools/dev/labels.sh table                       # the same rows, for a script
tools/dev/labels.sh check                       # or: task lint:labels
tools/dev/labels.sh sync --dry-run              # what declaring them would do
tools/dev/labels.sh sync                        # create and update them on GitHub
tools/dev/labels.sh sync --prune --dry-run      # ...and which labels it would delete
printf '### Platform\n\nWindows\n' | tools/dev/issue-labels.sh   # the labels a body asks for
```

**`tools/dev/labels.sh` is the one place a label is written down**, and `sync` is what puts the table
on GitHub. `--prune` deletes every label the repository carries that the table does not name. That is
a change to GitHub rather than to a checkout, so run it behind `--dry-run` first.

**`.github/workflows/issue-labels.yml` applies the platform and program labels** from the bug form's
own dropdown answers, when an issue opens and when its body is edited. `tools/dev/issue-labels.sh` is
what it runs, and it reads a body on stdin, so it is tried on a hand-written one without opening an
issue. The standing decision is
[`An issue carries the platform and the program it is about`](docs/decisions/repository.md#an-issue-carries-the-platform-and-the-program-it-is-about).

## Releases

Everything lands under `dist/<app>/<platform>/`. **Video is on by default in every staging script**,
and `--no-video` asks for the smaller build. A script that cannot find ffmpeg **stops before it
builds anything**. They are quiet, and `-v` watches one — **a step that fails replays everything it
held back.**

```sh
task dist                     # everything this platform can carry
task dist:app                 # ...just the machine
task dist:tools -- km-pack    # ...just these tools
task dist:setup               # ...the setup program: one installer, every product
task dist:setup:notarized     # ...that one signed and notarized, on macOS
task dist:setup:remote        # ...the remote alone, in a small installer of its own
task dist NO_VIDEO=1  ZIP=1  VERBOSE=1
```

| Script | Produces |
|---|---|
| `tools/platform/windows/dist.sh` | the portable folder: two exes, four ffmpeg DLLs, assets |
| `tools/platform/macos/app-bundle.sh` | `Karaoke Machine.app` — must run on macOS |
| `tools/platform/linux/deb.sh` | a Debian 13 `.deb`, built in Docker, carrying its own ffmpeg |
| `tools/platform/linux/deb.sh --system-ffmpeg` | the same, linking Debian's ffmpeg instead |
| `tools/platform/linux/deb.sh --tools` | `karaokemachine-tools`: the package builder, the remote and km-admin |
| `tools/platform/linux/tarball.sh` | a portable folder + `.tar.gz`, built in Docker |
| `tools/dist/cmd.sh` | the six command-line tools, one folder each |
| `tools/dist/bin.sh` | one folder with **every** executable in it |
| `tools/platform/windows/installer.sh` | one Inno Setup program carrying all seven products |
| `tools/platform/macos/installer.sh` | the same as a `.pkg` |
| `tools/platform/windows/installer-remote.sh` | a second, small one carrying the remote alone |
| `tools/platform/macos/installer-remote.sh` | the same as a `.pkg` — must run on macOS |
| `tools/port/machine/ios/build.sh --ipa` | the machine as an unsigned `.ipa` — must run on macOS |
| `tools/port/remote/ios/build.sh --ipa` | the offline remote as one |

- **The `-no-video` marker goes on the declined build**, because the plain name should name what the
  plain command produces. The `.deb` uses a `no-video/` subfolder instead, both builds having the same
  filename.
- **`tools/dist/bin.sh` and the two installers gather rather than build**, so nothing about features,
  DLLs or READMEs is written down twice. They are deliberately **not** part of `task dist`, which would
  otherwise stage everything twice.
- **All four installers round-trip themselves on every build** — install, run every executable on a bare
  PATH, uninstall, assert the directory is gone. The macOS pair expands the archive instead, a
  system-domain install needing root; `--install` does the real thing.
- **The remote's two are independent carriers**, not smaller selections of the other two. They have
  their own `AppId` on Windows and their own receipts on macOS, so one computer may hold both, and
  removing either leaves the other. The Windows build refuses when the two `AppId`s match.
- **A setup program's name carries the system it installs on**, `-windows-` or `-macos-` before the
  architecture. All four sit in `dist/setup/<platform>/`, where the folder says it. A release page is
  flat, and somebody choosing a download there has the filename and nothing else.
- **Nothing signs the four by default**, so a recipient sees SmartScreen or Gatekeeper. Signing the
  macOS ones takes **two** certificates, and `--notarize` is never implied by signing — `task
  dist:setup:notarized` and `task dist:setup:remote:notarized` are the names for asking. **Windows
  has no equivalent** and no signing at all: that needs a certificate nobody here has, so neither
  Windows installer takes such a flag.

```sh
tools/platform/linux/verify-deb.sh          # install that .deb in a clean container
tools/platform/linux/verify-deb.sh --system-ffmpeg   # ...the one from system-ffmpeg/
tools/platform/linux/verify-deb.sh --tools           # the tools package, installed beside the machine
tools/platform/linux/verify-tarball.sh      # unpack + run it in a clean container
tools/platform/linux/verify-tarball.sh --image fedora:42 | --bare | --refresh
tools/platform/linux/prewarm.sh [--all|--check|--refresh]
```

**The `.deb`'s dependency closure is deliberately never cached**, because that verifier asserts the
`Depends` list is *complete*. That only holds because the image starts with nothing. The tarball's
runtime list *is* prewarmed, because it is a fixed list unrelated to the artifact under test.

```sh
tools/dist/check-assets.sh          # what a staging run is about to carry
tools/dist/clean.sh --all           # every staged release goes         (task clean)
tools/dist/clean.sh --old --dry-run # ...only the stale ones            (task clean:old)
tools/dev/clean.sh [--docker --cache]  # ...and everything else a build wrote (task clean:all)
```

**`--docker` and `--cache` are the two things that are not only yours.** A Docker prune takes a
parallel session's cache with it, and the asset cache is a download plus an ffmpeg build to refill.
Neither is therefore ever implied. **`local/`, `scratch/` and `.wpcache/` survive every setting.**

## Running what was staged

```sh
task run                        # the staged machine; -- args forwarded
task run WAIT=1                 # ...and wait for it
task run CONSOLE=1              # ...the Windows twin that prints (waits)
task run:package-builder -- /path/to/songs
task run:remote -- --machine 192.168.1.5    # the offline remote, staged
task run:assets                             # the asset finder, staged
```

**They launch and return.** What is being started is a karaoke machine somebody uses for an evening, so
blocking cost the terminal until the singing stopped. A detached run's output is discarded; run it
again with `WAIT=1` if it will not start.

## Android and iOS

```sh
task build:android                    # all four steps, out comes app-flat-debug.apk
task build:android RELEASE=1 | ARM64=1 | NO_VIDEO=1
task build:android:quest              # the headset APK, out comes app-headset-debug.apk
task build:android:native             # ...the cargo half, no Gradle and no JDK
task build:android:remote             # the offline remote, a much smaller APK
task build:ios                        # the machine as an .app, assets and ffmpeg included
task build:ios RELEASE=1 | DEVICE=1 | NOAPP=1 | NO_VIDEO=1
task build:ios:native                 # ...the cargo half, no xcframework and no project
tools/port/remote/ios/build.sh [--release] [--device-only] [--no-app]
```

- **`ARM64=1` is not a thing to ship.** Every Google TV device runs a 32-bit OS and loads
  `armeabi-v7a` alone. An APK without it installs on a phone and fails on a television.
- **`JAVA_HOME` must name a JDK 17 or 21.** The one already on a machine with Android Studio is the
  bundled JBR, which is the version Gradle refuses.
- **Songs reach a device through a public folder** that needs no permission and takes a plain
  `adb push` with no `run-as`. `--show-paths` names both.
- **`adb logcat -s karaokemachine`** for the machine, `-s km-remote` for the remote. **iOS writes to
  stderr**, which Xcode's console shows, so there is no equivalent command and no bridge crate.
- **Never edit either iOS project in Xcode** — both are generated, and the next build discards the
  change.
- **The two iOS builds are the only tasks here that cannot run on Linux or Windows.** They carry
  `platforms: [darwin]`, and a task whose platform does not match is **skipped rather than failed**.
  A green line on another host therefore says nothing about whether the app compiles.
- **`task build:ios` fetches two things on its first run**: the lyric font, and an ffmpeg it
  cross-compiles for the phone. Both land in the shared asset cache, so a second worktree pays
  neither.
- **`NO_VIDEO=1` leaves out the decoder and the four embedded frameworks with it**, which is what a
  quick iteration wants. The machine then refuses a video song exactly as a `--no-video` desktop
  build does.

**Getting the result onto a device is [`DEPLOYING.md`](DEPLOYING.md)** — finding the device with
`adb` first, then installing, and the signing traps on each platform.

## Deploying to the appliance

```sh
task deploy:linux HOST=user@box              # tools/platform/linux/deploy.sh user@box
task deploy:linux HOST=user@box NO_BUILD=1 | NO_VIDEO=1 | SONGS=./packages
task deploy:linux HOST=user@box PORT=2222 | IDENTITY=~/.ssh/karaoke
task deploy:linux:boot HOST=user@box         # tools/platform/linux/appliance-boot.sh user@box
task deploy:linux:boot HOST=user@box REVERT=1 | FORCE=1 | SLIM=1      # once per box
tools/platform/linux/grub-appliance-edit.sh < /etc/default/grub        # the filter the above uses; testable alone
```

Builds, copies, installs and enables the service. **`HOST` is the one variable in the Taskfile with
no default.** The value is an address of somebody's own house, and no tracked file may carry one. It
names the *sudo* account on the box, never the service account. `SONGS` sends each `*.kmpkg` into the
machine's packages folder, and restarts the machine so it reads that folder again. A package is one
file, so there is nothing beside it to leave behind.

`deploy:linux:boot` is the **once-per-box** second half. It hides the bootloader menu behind a
one-second any-key window, and selects the Plymouth theme the package installed but did not switch
on. It needs the package to be there already, it asks for a password where a deploy does not, and
`REVERT=1` puts the box back. See
[`What the box shows before the machine does`](docs/decisions/distribution.md#what-the-box-shows-before-the-machine-does).

**[`DEPLOYING.md`](DEPLOYING.md) is the reference.** It has the requirements at both ends, and the
display manager that causes a black screen. It also says why the journal is read with `-t` rather
than `-u`.

## Working in parallel

```sh
tools/dev/worktree.sh video-fix            # .claude/worktrees/video-fix
tools/dev/worktree.sh --remove video-fix   # refuses if dirty; leaves the branch alone
```

**This repository asks for worktrees**; see `CONTRIBUTING.md` for why, and for the two collisions that
are not git's problem.

## The wallpaper pack

**Read this first: which source you search decides whether the pack may be shared.** Openverse packs
may be handed on. **A Pixabay or Pexels pack is for the machine that built it**, both providers'
terms forbidding the distribution of content as a wallpaper. Such a pack is never committed, and
never staged into a release, whoever types `--zip-dest`.

It is in the second workspace, `tools/cmd/assets`, so every command names its manifest. That is the
**workspace root**, with `-p` picking the program, which is what keeps one spelling working as
members are added. Keys come from the environment and never from the config file.

```sh
W="tools/cmd/assets/km-wallpaper-pack"
cp $W/config.example.toml $W/config.toml
cargo km-wallpaper-pack all     --config $W/config.toml
cargo km-wallpaper-pack analyze --config … [--remeasure]   # re-tune, no network
cargo km-wallpaper-pack build   --config … --zip-dest ./out
cargo km-wallpaper-pack verify  --pack ./out               # exits 2 on a contrast failure
cargo km-test-assets
```

**The alias is `--release`, and that is not a default to override.** `analyze` is thousands of JPEG
decodes, resizes and blurs; over a full cache an unoptimized run took 78 minutes against single-digit
ones optimized. It is carried in `.cargo/config.toml` for the reason `cargo km` carries `video`. A
debug build here is not a cheaper answer to the same question; it is the wrong answer slowly.

`fetch` is the only phase that touches the network. `analyze` and `build` are pure functions of the
cache, so thresholds can be tuned without spending quota. Measurements are cached too, keyed on the
output size and the legibility table and **deliberately nothing else** — so re-tuning a filter
re-decodes nothing. `--remeasure` covers the one invalidation that key cannot see: an edit to the
measuring code itself.
