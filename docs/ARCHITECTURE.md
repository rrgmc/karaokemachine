# Architecture

How KaraokeMachine is built. This is the overview — the language and toolkit choices, the crate
layering, and the threading model. The detail is one file per area under
[`docs/architecture/`](architecture/).

Two neighbors worth knowing before you start:

- **[`docs/decisions/`](decisions/) says *what was decided and why*** — product-level choices, each
  one an entry with its reasoning. This file and its children say *how the thing is built*. When the
  two seem to disagree, the decision is authoritative and the architecture note is out of date.
- **[`BUILDING.md`](../BUILDING.md) says how to compile it**, and
  [`CONTRIBUTING.md`](../CONTRIBUTING.md) what to run before a pull request.

## The detail, by area

| File | Covers |
|---|---|
| [`song.md`](architecture/song.md) | `km-song` parsing, encoding detection, `km-suitability` melody and the rubric, `km-fixes` per-song corrections, language as a code |
| [`audio.md`](architecture/audio.md) | `km-audio` synth and sequencer, the SoundFont, the queue/synth split, ALSA `dmix`, the audio period the display rides on |
| [`packaging.md`](architecture/packaging.md) | `km-kmpkg`, `km-catalog`, how a package is really made, a package is one file |
| [`package-builder.md`](architecture/package-builder.md) | the curation tool, the largest single area here |
| [`video.md`](architecture/video.md) | `km-video` decoding, curating videos |
| [`stream.md`](architecture/stream.md) | `km-stream` encoding, the watch page, and what is silent when it is wrong |
| [`cdg.md`](architecture/cdg.md) | `km-cdg`, packaging and curating MP3+G |
| [`ultrastar.md`](architecture/ultrastar.md) | UltraStar songs: the parser, the timeline entry, finding the audio, playing it on the audio clock |
| [`songbook.md`](architecture/songbook.md) | `km-songbook`, the printed book |
| [`api.md`](architecture/api.md) | `km-api` — control surface, admin mode, discovery |
| [`display.md`](architecture/display.md) | `km-display`, the now bar, the browse bar, the keypad, the queue overlay, the lyric timing offset |
| [`remote.md`](architecture/remote.md) | the two remotes, the portable core, the Android and iOS shells, the connection budget, carrying favorites off a phone |
| [`admin.md`](architecture/admin.md) | the owner's page and its two hosts, the trait-per-tab seam, `AdminError`, which types cross it and which does not |
| [`desktop-shell.md`](architecture/desktop-shell.md) | the icon in the bar, the event loop, the webview's profile |
| [`persistence.md`](architecture/persistence.md) | settings, catalog, what is written where |
| [`android.md`](architecture/android.md) | the APK, the armv7 `cdylib`, assets in the APK, video and packages on Android |
| [`ios.md`](architecture/ios.md) | the machine on a phone: SDL from source for iOS, the frameworks the app binary is given, the paths handed down from Swift |
| [`assets.md`](architecture/assets.md) | the asset directory and local overlay, the application icon, wallpapers, `km-wallpaper-pack`, the README's pictures |
| [`appliance.md`](architecture/appliance.md) | Debian on a bare TTY, and the three cold-boot faults found on it |
| [`distribution.md`](architecture/distribution.md) | the carriers — folders, bundles, `.deb`, tarball, both installers — and what a staging run says |
| [`carols.md`](architecture/carols.md) | the Christmas carol pack and its license gate |

## Context

The brief: a native cross-platform
app (Windows/macOS/Linux first, Android second), synced word highlighting, a control API for
search/queue/transport/tone, song packages with hardcoded queue numbers, and packaging tools.
Explicitly **no scoring of singers**.

## Why Rust

- `midly` parses SMF with complete meta-event coverage, including `MetaMessage::Lyric` and
  `MetaMessage::Text` — both are needed, since Soft Karaoke `.kar` files put lyrics in *Text* events
  while standard MIDI karaoke uses *Lyric* events.
- `rustysynth` is a pure-Rust SoundFont synthesizer (a port of MeltySynth) with no dependencies
  beyond std. `Synthesizer` is `Send + Sync` and exposes `note_on`/`note_off`/
  `process_midi_message`/`render(&mut [f32], &mut [f32])`, so it can be driven directly from an audio
  callback by our own sequencer. Reverb and chorus are built in. **It is taken from a git branch
  rather than from crates.io** — the only dependency here that is, for SF2 modulators and lenient
  bank loading; see [`The synthesizer is a fork`](decisions/audio.md#the-synthesizer-is-a-fork).
- Both are pure Rust, so the whole audio path is one `cargo build --target ...` per platform with no
  C toolchain, and Android becomes `cargo-ndk` rather than a per-platform CMake problem.
- Go was rejected: `go-meltysynth` and `gomidi` exist and are pure Go, but a GC'd runtime inside a
  real-time render callback is a genuine dropout risk, and every GUI option needs cgo, which forfeits
  Go's cross-compilation advantage.
- C++ has the widest library selection (FluidSynth, JUCE, SDL3 natively) but the worst
  dependency-management story and manual memory management on the audio thread.

## Why SDL3

SDL3 is the right tool for a full-screen, game-loop, GPU-accelerated karaoke display, and it has the
best first-class Android support of the candidates. `sdl3-rs` (0.18.x) is actively maintained
and exposes `ttf`, `image` and `build-from-source-static` features, so SDL3 itself can be vendored
and statically linked — no system SDL install on any platform.


## The workspace

A Cargo workspace. Each crate is independently testable; only `km-display` needs a GPU and only
`km-audio` needs an audio device.

```
Cargo.toml                 # workspace
CLAUDE.md                  # repo rules + pointers to the two plan docs
crates/
  song/                    # what a song is, and the catalog of them
    km-songcode/           # how a singer names one: a bank and a slot. A leaf taking
                           # serde + thiserror, because the
                           # catalog, the queue, the API, the display and both remotes all have
                           # to name these types and no other crate is visible to all of them.
    km-song/               # SMF -> Song model; karaoke-format detection & lyric normalization
    km-suitability/        # melody-channel detection + 0-10 suitability scoring (packaging-time)
    km-fixes/              # per-song corrections for a file's own MIDI defects: detected at
                           # packaging time, resolved to a channel table at playback
    km-kmpkg/              # .kmpkg container read/write
    km-catalog/          # SQLite/FTS5 index over installed packages; search + number lookup
    km-songbook/           # the printed song book: a tiny PDF writer + the layout of a song list.
                           # `km-songcode` and NOTHING else -- no PDF crate, no font crate, no
                           # compression -- because the base-14 fonts need none of the three.
  playback/                # turning one into sound and picture
    km-audio/              # audio thread: rustysynth + our sequencer, transpose, the audio feed.
                           # The ONLY crate that names rustysynth or cpal.
    km-cdg/                # MP3+G songs: a CD+G renderer and an MP3 feed. Pure Rust, NO feature.
    km-video/              # video decoding -- the only crate that DECODES with ffmpeg. Optional.
    km-stream/             # ...and the only one that encodes with it: a drawn screen and the sound
                           # beside it into one HLS stream, for a television somewhere else. The
                           # split is by direction and neither takes the other. Its `pixels` module
                           # is outside the feature, because converting a screen into the planes an
                           # encoder wants needs no encoder -- swscale is off by build decision, so
                           # that conversion is written here, and it is tested on a machine with no
                           # ffmpeg at all.
    km-display/            # SDL3 renderer: wallpaper, lyrics, number entry, queue, connect panel
  machine/                 # the machine itself
    km-queue/              # what the machine is DOING: queue, mic registry, transport, playback
                           # limits. A leaf taking `km-songcode` and `thiserror`, so that
                           # describing a queue does not mean linking rustysynth -- see
                           # "The queue and the synthesizer are two different crates".
    km-banks/              # the 63 General MIDI banks this project knows how to fetch, compiled in
                           # from `data/soundfont-banks.conf`, plus the rules for checking one that
                           # arrives: which hash a row is pinned with, how far past the stated size
                           # a body may be, and how a bank published inside a zip is taken out.
                           # A leaf taking `sha1`, `sha2` and `zip` and nothing else -- **which is
                           # the point**: `tools/cmd/assets/km-admin` reads it from outside the
                           # workspace, so anything with TLS in it here would defeat the exclusion.
    km-api/                # axum HTTP + WebSocket server, the admin prefix, discovery advert
    km-admin-pages/        # the owner's page at `/admin/`: packages, pictures, sound, the
                           # machine itself, and what it found and could not use. **One page set
                           # behind two hosts** -- this machine through `in_process`, and
                           # `tools/cmd/assets/km-admin` over HTTP -- with a trait per tab as the
                           # seam. Nothing in its handlers names `ApiState`, which is what makes
                           # the second host possible; see docs/architecture/admin.md.
                           # Under machine/ and not remote/ because it is not a remote: it
                           # configures the box rather than driving a performance, and it is the
                           # machine's own surface even where a tool serves the same markup.
                           # Its guard DENIES by default -- where the surface faces a LAN -- which
                           # is the inverse of km-remote-pages': an unlisted route there is a
                           # favorite, and here it is a control-panel button somebody forgot to
                           # gate.
    karaokemachine/        # the binary -- wires audio + api + display. TWO binaries on Windows:
                           # `karaokemachine` (GUI-subsystem, double-click it) and
                           # `karaokemachine-console` (the one that prints). Hence src/cli.rs:
                           # `#![windows_subsystem]` is a property of a binary crate root, so the
                           # command line cannot live in either one's main.rs. Its *library* target
                           # is `km_app` -- the tracing target, and `libkm_app.so` on Android.
    km-machine-ios/        # a staticlib the app binary links: the entry point and the two
                           # directories Swift hands down. A staticlib for the reason
                           # km-remote-ios is one, and a shim crate so that the desktop and
                           # Android crate above keeps its two crate-types.
  remote/                  # the singer's remote: one set of pages, five hosts
    km-remote-pages/       # askama templates + htmx handlers serving BOTH modes, with
                           # `Capabilities` as the seam. Linked into karaokemachine for the
                           # online remote at `/` and into km-remote for the offline one.
    km-remote-core/        # the offline remote as a library: mirror, favorites, machine client,
                           # discovery, and a server with no `main` in it
    km-remote-host/        # what a host has to drive: the four phases, the process-wide
                           # singleton, the non-blocking stop. No platform in any of it.
    km-remote/         # the desktop shell: clap, directories, km-console, tokio::signal
    km-remote-android/     # a cdylib the APK loads; six JNI functions and nothing that thinks
    km-remote-ios/         # a staticlib the app binary links; six extern "C" functions. A
                           # staticlib because iOS forbids fork and exec.
  platform/                # what a host asks for, rather than what karaoke does -- the operating
                           # system, and the person in front of it
    km-console/            # is anybody reading? -- the only whole-module `unsafe` allowance, two
                           # calls. Shared by the machine, km-package-builder and km-remote:
                           # all three have a GUI-subsystem executable on Windows, where
                           # `println!` on a null handle is a panic.
    km-tray/               # an icon in the OS icon bar. Takes no `tao`: the caller owns the event
                           # loop and passes a closure, so this is not a second window library.
    km-webshell/           # where a tao/wry window opens: the clamp to the screen and the centering
                           # on the monitor, which were byte-identical in all three shells. Takes
                           # `tao` and *not* `wry` -- deciding where a window opens should not drag
                           # a browser engine in. Deliberately holds no event loop: each shell's
                           # differs in its `Wake`, its tray items and what ends a run.
    km-osopen/             # handing a file or a URL to whatever the OS uses for it. No
                           # dependencies at all. A crate rather than a copied file because
                           # there are three callers.
    km-androidlog/         # tracing events into logcat, the only place a line goes on a device.
                           # Takes a tag, because there are two Android applications.
    km-logfile/            # ...and the desktop counterpart: those same events into a file, for the
                           # three programs here that can be double-clicked into having no console.
                           # A MakeWriter and a retention policy; no dependency but tracing.
    km-logtap/             # ...and the third destination: the last few hundred records in memory,
                           # for `/admin/logs` to serve and `/dev/` to draw. A `Layer` where the two
                           # above are MakeWriters, because a pane wants the parts -- a level to
                           # colour by, a target to filter on -- where a file wants a line. Not code
                           # in km-api: a Layer is a subscriber implementation, and that crate keeps
                           # tracing-subscriber to its dev-dependencies so nothing describing a
                           # remote links one.
    km-ecapplog/           # ...and the fourth: that stream into the ECAppLog viewer, live, over a
                           # loopback socket. The one a person watches while a run happens, where
                           # the three above serve a terminal, a folder and a browser. Takes
                           # `--ecapplog`'s address and hands back a Layer; the console's is not
                           # built when it does. Four programs assemble it, two of them from the
                           # other workspace, which is why the address parser and the rule filing a
                           # record under its crate live here rather than four times over.
    km-logsettings/        # what a settings file says about all four of those: the `logging`
                           # section, its two either-shape keys, and the peek that reads it before
                           # a subscriber exists. Read by the three programs that keep a settings
                           # file and can be started from an icon -- the machine, the package
                           # builder and km-admin. A crate on km-logfile's axis: the `-v` ladder
                           # stays duplicated because its rungs name a different crate in every
                           # program, and a grammar does not, because three copies of one would be
                           # three answers to what a key accepts.
    km-locale/             # which language a surface speaks, and the machinery for saying it: the
                           # locale, Accept-Language negotiation, a Fluent catalog and an askama
                           # filter. **No product words** -- every catalog lives in the crate whose
                           # words they are, `include_str!`-ed. See `Catalogs live beside the words
                           # they translate`.
tools/                     # six folders and nothing loose -- see "How things here are named"
  cmd/                     # the commands somebody types. A path here is never typed: what selects
                           # one is its package name, `-p km-pack`.
    km-pack/               # lib: the packaging pipeline, shared with km-package-builder
                           # bin: build/inspect/validate .kmpkg packages (runs km-suitability)
    km-lyrics/             # CLI: dump a parsed lyric timeline + analysis as JSON (debugging)
    km-package-builder/    # the curation tool: a local web server over a folder of source files,
                           # with a Fluent catalog of its own in i18n/
    assets/                # **a second workspace**, excluded from the one above because these two
                           # need TLS from reqwest and km-package-builder needs it to have none.
                           # One root for the pair: one lockfile, one CI job, one fmt line.
      km-wallpaper-pack/      # builds a legibility-verified wallpaper pack
      km-admin/           # KaraokeMachine Admin: a local web server that finds pictures (through
                           # km-wallpaper-pack's own three phases) and SoundFont banks (through the
                           # km-banks table), and uploads them to a machine. The fourth product,
                           # and the answer to a machine with no internet and a television box
                           # with no shell.
                           # **It also HOSTS the owner's page** rather than carrying a copy: it
                           # implements `km_admin_pages::machine`'s traits over its HTTP client and
                           # serves that crate's markup at `/admin`, with its own searching merged
                           # beside it. It used to keep a second layout, machine page and output
                           # picker in step with the machine's by hand; see
                           # docs/architecture/admin.md.
  port/                    # building the native shells. Mirrors ports/ exactly, so the answer to
                           # "which scripts build this?" is the same path read twice.
    ndk.sh                 # finds the Android NDK. Here rather than under machine/android/ because
    libc_n.map             # both Android ports use them -- the map via a cfg(target) rustflag.
    machine/android/       # build, stage, assets, ffmpeg -- the machine as an APK
    remote/android/        # build, stage -- the offline remote as an APK
    remote/ios/            # build -- the xcframework and the generated Xcode project
  platform/                # what a platform asks for, rather than what karaoke does. The same word
                           # crates/platform/ uses, for the same reason.
    linux/                 # the .deb, the tarball, their verifiers, the build image, and deploy.sh
    macos/                 # the .app bundle, two .pkg drivers over one pkg.sh, two uninstallers,
                           # four Info.plist
    windows/               # the portable folder, and two Inno Setup installers over one inno.sh
  dist/                    # producing something to hand over
    common.sh              # sourced by fourteen callers; asserts the caller reached the repo root
    bin.sh cmd.sh clean.sh check-assets.sh
  dev/                     # the working session
    worktree.sh claude-worktree-hook.sh screenshots.sh check-no-local-refs.sh
    clean.sh               # the whole checkout, where dist/clean.sh is the releases in it
    soundfont.sh soundfont-debug.sh soundfont-measure.sh
    km-pick/               # the one crate under dev/: a checkbox list over `inquire`, so a shell
                           # script can ask for a choice. Knows nothing of what it is listing.
    remote/                # dev web remote: one static HTML page + curl/websocat scripts. Staged
                           # as `remote-dev/` beside an executable -- see the note on that name.
  setup/                   # what a build here needs, and where it comes from
    fetch-assets.sh fetch-ffmpeg.sh ffmpeg-pin.sh features.sh
    asset-cache.sh         # where the fetched things live -- one definition, four readers
ports/                     # the native application shells: machine/android, remote/android, remote/ios.
                           # Gradle and Xcode projects only -- the Rust is in crates/.
icon/                      # generated application icons. NOT under assets/, because nothing reads
                           # them at run time.
assets/
  fonts/                   # Latin-coverage UI/lyric font
  soundfont/               # bundled GM bank (fetched by tools/setup/fetch-assets.sh, not committed)
  wallpapers/              # a few defaults so the app looks right on first run
docs/
  ARCHITECTURE.md          # this file: the overview and the crate map
  architecture/            # one note per subsystem, and what was measured building it
  decisions/               # why something is the way it is, in topic files, indexed
  research/                # investigations -- findings, never commitments
  images/                  # the screenshots the README links
  learning-rust.md         # what a C++ reader needs to read this codebase
```

## How things here are named

**A crate is named for what it holds, not for the category it belongs to** — and, where a category
has several members, for **which** of them it is. `km-suitability` rather than `km-analyze`, which is
a verb with no object; `km-catalog` rather than `km-library`, because "library" is every crate here;
`km-audio` rather than `km-engine`, an engine of what.

**The `km-` prefix stays on all of them.** Cargo's package namespace is flat; `use km_song::` is
what separates a workspace crate from a third-party one at a glance in every file; and the bare
alternatives — `song`, `api`, `audio` — are names other people's crates already have.

**`crates/`' five folders are the dependency layering, written down.** The graph is six clean layers,
and a flat listing of twenty-two peers says none of that; worse, six of those names begin
`km-remote-`, so an alphabetical listing reads as a third remote by volume. The folders are
organizational and enforce nothing — `remote/km-remote-pages` depending on `machine/km-api` is
correct.

**`tools/`' six folders are the stages of the work**, because a scripts directory has no dependency
graph to sort by and does have a sequence: get a machine ready (`setup/`), build a shell (`port/`),
satisfy a platform (`platform/`), hand something over (`dist/`), work on it (`dev/`), plus the things
that are not scripts at all (`cmd/`). Two of the six are named to be recognized rather than read:
`port/` mirrors `ports/` **exactly**, and `platform/` is the same word `crates/platform/` uses for the
same reason. **Nothing is loose at the top level** — a script with no folder is a script nobody can
place.

**A path under `cmd/` is never typed**: a package is selected by name (`-p km-pack`). The one
exception is `--manifest-path tools/cmd/assets/km-wallpaper-pack/Cargo.toml`, because that crate is
excluded from the workspace and therefore has to be addressed by path.

### A crate name and a runtime identifier are not the same thing

They look identical in a grep, and only one of them is free to move.

- **`km_app` is the machine's library target**, spelled out in the manifest because
  `karaokemachine` would collide with the binary's output name. It is the tracing target in
  `info,km_app=debug`, the `libkm_app.so` the APK stages, and the `System.loadLibrary("km_app")`
  Java calls.
- **`karaokemachine` is a Debian package name, a systemd unit, `/opt/karaokemachine` and an
  on-`PATH` command.** Renaming it would orphan every installed unit on upgrade.
- **The remote's data directory is `km-remote`** — `ProjectDirs`, `%APPDATA%\km-remote`,
  `~/Library/Application Support/km-remote` — and moving it would orphan a `favorites.sqlite`
  somebody built up over a year. A *file* inside that directory can be renamed by the code that
  opens it, because that code knows both names; a moved *directory* is one nothing is left looking
  in.
- **`tools/dev/remote/` is staged as `remote-dev/`**, beside a staged executable and inside the
  APK's assets, because that is a directory the machine *looks for* at run time and
  `api.dev_remote_dir` defaults to. Renaming the deployed name would make every installed machine
  look for a folder that is not there.
- **`include/km_remote.h`** belongs to `km-remote-ios` and is a C header other people compile
  against.

### A tracing filter names a target the compiler never checks

**A filter naming a target that does not exist is an error nowhere.** It parses, it does not warn,
and a test asserting the string still passes; `-v` simply stops printing.

So those names are derived rather than typed — `env!("CARGO_CRATE_NAME")` for a crate's own target,
and an exported `LOG_TARGET` from `km-remote-pages`, `km-remote-core` and `km-api` for the ones named
from elsewhere (`CARGO_CRATE_NAME` answers only for whoever is compiling, and an example is its own
crate). `km-remote-core` re-exports the pages' as `PAGES_LOG_TARGET`, because none of the three
mobile and desktop shells depends on that crate directly and a dependency edge is too much to add for
a string. **The tests keep their literals on purpose**: derived in the code and spelled in the test,
a rename fails loudly in one place rather than silently in none.

### What a rename or a move has to touch

The compiler covers every `use` statement, and it covers nothing below.

**The cross-file consistency obligations:**

- **The three copies of the feature string** — `.cargo/config.toml`, `tools/setup/features.sh`,
  `BUILDING.md`. `tools/platform/linux/check.sh` asserts the first two match *literally*, and
  `ci.yml` runs the aliases.
- **The fifteen `-p <package>` aliases** in `.cargo/config.toml`, plus the `-p` sites in
  `Taskfile.yml` and the staging scripts.
- **The `assets` path filter in `ci.yml`'s `changes` job.** It **fails open**: after a move of
  `tools/cmd/assets` it matches nothing, the job skips, and CI is green because nothing ran. The
  weekly run still covers it.

**The escapes out of a crate**, which are the loud ones: `include_bytes!`/`include_str!`,
`build.rs` icon paths, and `CARGO_MANIFEST_DIR` joins. All but the last fail at compile time, so
`cargo km-build` is the proof.

**Every script's `cd` to the repository root is a hard-coded count of `..`**, two, three or four
depending on the folder, across twenty-nine scripts. A wrong count lands somewhere plausible and the
failure surfaces later wearing the face of whatever ran next — so `dist_assert_root` is asserted at
the top of `dist/common.sh`, which is the first thing fourteen callers reach after their `cd`, and
the six that deliberately do not source it carry the same two lines inline. **A new script under
`tools/` must do one or the other.**

**Three more with no compiler and no test behind them:**

- `.cargo/config.toml`'s `--version-script=tools/port/libc_n.map`, reached only by the armv7 Android
  link, so only an Android build says whether it is right.
- `linux/image-tag.sh` hashes the `Dockerfile` and `apt-deps.sh` **by literal path** to name the
  build image, so the tag changes and the first `check.sh` or `deb.sh` afterwards rebuilds it once.
- `port/machine/android/ffmpeg.sh` brands a block in `$CARGO_HOME/config.toml` with **its own path**.
  Stripping it by exact match orphans the last block and inserts a second `[env]` key beside it — a
  config cargo refuses to read at all, so not an Android build broken but every build on that
  machine. It matches the marker's stable *prefix* instead.

**And the file lists that name a product by hand:** `icon/`'s files and the `include_bytes!` sites
that name them — one per program, plus the machine's badged mark in `src/tray.rs` and
`src/register.rs`, `tools/platform/macos/Info.*.plist` (`CFBundleExecutable` and
`CFBundleIconFile` are strings Xcode never sees), the `ALL_APPS` and `ALL_TOOLS` arrays and per-tool
`case` arms in `tools/dist/bin.sh` and `tools/dist/cmd.sh` plus the README prose `cmd.sh` generates,
`installer.iss`'s `Source:` and `Name:` lines, and the literal tool lists in the macOS installer and
uninstaller.

### Do not sweep the documentation

**A rename's own `sed` must not touch the text that documents the rename.** Three `tools/` strings in
this repository are deliberately not the current path and each would otherwise become a lie: the
Android SDK's own `[sdk]/tools/proguard`, the `.pkg` payload path in `macos/installer.sh`, and
`tools/km-package-builder/src/console.rs` in `km-console` — that sentence says where the module
*began*, and no file ever existed at the current path.


## Threading and data flow

```
SDL3 display thread --+                    +-> AtomicU64 position (tick<<8) -> display
                      +-> Command queue -->|   AtomicU8  transport state
axum/tokio (API) -----+   (rtrb SPSC)      +-> event channel -> API WebSocket
                                    |
                          control thread (queue state machine, song loading, persistence)
                                    |  Arc<Song>
                                    v
                          cpal output callback  <- the only real-time thread
                            (opened on demand, dropped when idle)
                            drain commands -> advance sequencer -> synth.render()

wallpaper loader thread -> decoded+downscaled RGBA -> display thread uploads texture

video decoder thread (one per loaded video song, km-video)
   |-> interleaved f32 at the FILE's rate -> rtrb ring -> TrackPlayer resamples in the callback
   +-> pooled YUV420 frames + pts -> bounded channel -> display uploads to an IYUV texture
```

Hard rules: no allocation, file I/O, locking, or parsing in the cpal callback. Songs are parsed on
the control thread and handed over as `Arc<Song>`. State flows out through atomics (for the
per-frame position the display needs) and a channel (for discrete events the API broadcasts). Image
decoding never happens on the render thread.

**Demo mode adds no actor.** It is a deadline on the control thread's own state and three pure
functions over it, read by the same 20 Hz watchdog that already advances the queue — so "start a
random song after a minute of silence" is a comparison in `Machine::poll` rather than a timer
thread. That matters for more than tidiness: a separate thread would have to take the state lock to
decide whether a song is playing, and would then be racing the very transition it was waiting for.
The only new work off that path is one `ORDER BY RANDOM()` per demo song, which is a full table scan
and runs on the poll thread, not the callback. `PUT /api/v1/demo` is synchronous for the same
reason — it moves the deadline and returns, and the song starts on the next poll. **Reclaiming a
staged audition is the same convention's second user**: the sweep runs where the song is displaced,
and a folder that would not delete leaves a deadline behind for the same watchdog to pick up, so
nothing waits on a thread of its own for a file handle to be released.

**Advancing the queue is one transaction, and it needs a lock of its own.** The paragraph above
names the hazard — deciding whether a song is playing and then racing the transition you were waiting
for — and `Machine::advance` is where it lands. Three threads reach it: the 20 Hz watchdog when a
song ends, and any number of API threads through `queue_add` and `transport`. Reading
`loaded.is_none()`, dropping the guard and *then* popping lets the answer change between the two:
both pop, both parse, and the second `start` overwrites the first — one song playing, one gone,
nothing logged. Eight singers queueing at once leaves **one** of eight songs.

The state lock cannot fix it, and deliberately so: advancing releases it across the slow middle
(archive read, MIDI parse) precisely so a load does not stall the display or a search. So there is
a fourth mutex, `Machine::advancing`, guarding the *operation* rather than any data, held for the
whole of pop-parse-start and always taken before the other three. Callers who are replacing what is
on the deck — a song ending, and Skip — wait their turn; callers who are merely *choosing* a song
use `advance_if_idle`, or `advance_if_idle_or_over_a_demo` from `queue_add`, both of which give up
if the turn is busy, because a busy turn already means somebody else is starting the next song.

**The demo test in that second one is inside the turn lock for the same reason the idle test is.**
"Is the deck free?" has two answers that mean yes — nothing loaded, or a demo song loaded, which is
the machine filling a silence rather than anybody's turn — and both stop being true the moment the
lock is released, because the watchdog advances on its own whenever a song ends. Whether it took the
deck *from a demo* is returned rather than published inside, so `queue_add` is the one place that
decides a `SongEnded { yielded }` is owed.

**The stream comes and goes; the state object does not.** `SharedState` is created once by the audio
thread and handed *into* each `OutputStream::open`, because the machine holds that `Arc` for its
whole life — a reopen that made a fresh one would zero `songs_ended`, which the 20 Hz watchdog
compares against a count of its own, so it would read as a song having just finished and skip the
next one. The stream is closed only at `Transport::Idle`: `Stopped` and `Paused` mean a song is
loaded *inside the player the callback owns*, so closing would drop it while the screen went on
showing its title, and the next `Play` would reach an empty player and silently do nothing. Closing
joins cpal's worker thread, which is why it happens on the control thread and never anywhere near
the callback.


