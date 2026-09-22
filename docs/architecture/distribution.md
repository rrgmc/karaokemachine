# Distribution — the carriers

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## Where releases go

```
dist/<app>/<platform>/<the staged thing>
```

**App first, then platform.** A release is a thing you hand to somebody. So the useful grouping is
every build of one product together, rather than every Windows build of everything.

**`tools/dist/common.sh` is the one place that rule is written down**, sourced by every staging
script. `tools/dist/clean.sh` obeys it without sourcing and says why: every helper there *builds* the
layout, and that script is the only one that takes it apart.

Besides `dist_dir`, it holds the idioms those scripts had each spelled out:

| Helper | What it settles |
|---|---|
| `dist_dir` / `dist_staged_dir` | the layout, and *finding* a folder a staging script produced — exact name first, then a `<app>-<version>-*` glob, so a `-no-video` variant is still found |
| `dist_target_dir` | where cargo *builds*, asked of cargo rather than assumed to be `target/` |
| `dist_host_triple`, `dist_platform`, `dist_exe_ext` | `rustc -vV` rather than a hard-coded triple |
| `dist_version <exe>` | the version comes from the artifact, which cannot disagree with itself |
| `dist_clear <dir>` | clear a folder's *contents*, never the folder — Windows cannot remove a directory that is some shell's cwd, and every script tells you to `cd` into the one it just staged |
| `dist_zip_as` | a zip for a folder whose own name is not the archive's. **A zip's top-level entry is a directory somebody will look at**, so renaming the `.zip` afterwards does not fix a bad name — it is inside the file |
| `dist_ffmpeg_dir` / `dist_ffmpeg_check` | a *directory* on Windows, because four DLLs get copied out of it; a yes-or-no everywhere else, because nothing does |
| `dist_stage_app_licenses` | `LICENSE-MIT` and `LICENSE-APACHE`. **Unconditional, and an obligation** — MIT asks that the notice be in every copy, so a folder carrying only the words "MIT OR Apache-2.0" did not satisfy our own license. The `.deb` is the one carrier that skips it, because Debian policy owns `/usr/share/doc/<pkg>/copyright` |
| `host_path` | the Docker bind-mount path conversion under MSYS |

**Asking *where* ffmpeg is only makes sense on Windows**, because only Windows copies anything out of
that directory. Elsewhere the question is *whether* there is one. On macOS a perfectly good Homebrew
install has no DLLs in it. On Linux `pkg-config` finds a distribution ffmpeg while `--print-dir` has
nothing to print. The README each folder carries had the same fault, and it mattered more, because a
folder outlives the run that made it. `video_runtime_note` writes whichever of the two is true, and
off Windows it names the shared libraries to install.

Two scripts deliberately keep their own copy of something and say so in place.
`tools/port/machine/android/assets.sh` repeats the asset loop, because it copies into the APK's asset
*root*. `tools/platform/linux/check.sh` keeps `host_path`, because it stages nothing.

### The two slots that are not products

```
dist/bin/<platform>/           the windowed form of anything that has one
dist/bin-console/<platform>/   the console form; every single-form tool in both
```

- **It gathers; it does not build.** It runs the same staging scripts `task dist` runs and copies
  what they produced, so nothing about features, DLLs or READMEs is written down twice.
- **Its knowledge is three rules about shape, not a table.**
  1. A `*.app` is a windowed form, so the bare executable staged beside it is the console one.
  2. A file `<x>-console<EXT>` is a console form, so its `<x>` sibling is the windowed one.
  3. Everything else is single-form and goes in both.

  **The first two are one rule in two spellings.** Windows names the pair as two files, and macOS as
  a bundle beside the executable it wraps. Rule 1 had only its first half until somebody noticed that
  `dist/bin/macos` held `km-remote` and `KM Remote.app` side by side. `has_bundle` asks the staging
  directory rather than a list of products, and that keeps `--no-desktop` right for free. That flag
  stages no bundle, so the bare executable is single-form again and goes to both.

  `is_program` decides by **name, not the mode bit**. On Windows every file reports executable to a
  Git Bash `-x`. Off Windows the tarball's `install.sh` is 0755. So a mode-based rule would copy it in
  as a product and then *run it* during the checks.
- **Anything matching no rule is named, not dropped**, so the rules can stay narrow.
  `km-package-builder.exe.WebView2/` is the case that proves it exists.
- **A rule about shape cannot tell a bundle from its own fossil.** `gather_bundles` globs `*.app` in
  a product's staging folder. A rename leaves the old name sitting beside the new one, and nothing
  clears it. `dist_clear` empties the bundle it is about to *write* and cannot see a sibling.
  `tools/dist/clean.sh --old` matches a name followed by a version, and a bundle carries no version.
  So the glob gathered both, and the setup program refused the payload with
  `KaraokeMachine Package Builder.app, which no component claims`: the right answer to the wrong
  question.

  It also surfaced nowhere near its cause. The rename that made it moved three bundles on a machine
  with no `dist/` to fossilize. The failure arrived a day later on a machine that had staged before
  it. **The fix sits where the fossil is made rather than where it is found.**
  `dist_stage_macos_bundle` takes any other `.app` beside the one it is staging, unless the product
  has declared it in `DIST_MACOS_ALSO_STAGES`.

  The machine is the one product that declares anything. It stages a second bundle so that `--stream`
  has a launcher, and each of the two would otherwise sweep the other as a fossil. The setting is an
  array rather than a list, because two of the bundle names have spaces in them. Everything not
  declared is still a fossil and still goes. The rule is derived, so no dead name is written down
  anywhere, and the next rename needs no edit.

  It does not reach a whole product folder for a program that the build has dropped, such as
  `dist/km-assets/` or `dist/wallpaper-pack/`. Nothing stages into one to notice. That stays `--all`
  or a hand removal, argued above `sweep` in `clean.sh`.
- **Completing rule 1 moved files, and a consumer that read one folder lost them.** The macOS setup
  program took its whole payload from `dist/bin/macos`. That folder held every product's every form
  until the bundled three stopped leaving a bare executable there. It kept building: all four `.app`
  bundles were correct, so the package looked right and installed **four commands into
  `/usr/local/bin` instead of seven**.

  The lesson is not about macOS but about the shape of the change. A rule that *relocates* an output
  breaks readers that a rule which only *adds* one would not. And when `bin/` is a superset for long
  enough, readers treat it as one.

  `installer.sh` reads both folders. From `bin-console/` it takes
  whatever `claim()` claims that `bin/` did not supply. That set is derived rather than a list of the
  three names. **Windows was never affected.** There the pair is two files under two names, so
  rule 2 keeps `<x>.exe` in `bin/`.
- **Its own round trip is what caught that.** The round trip unpacks the built package and starts
  each of the seven commands out of the extracted payload. This is a property rather than an anecdote.
  The setup program is macOS-only, and a Mac builds it rather than the release workflow. So that check
  is the only thing standing between a payload rule changing and a release shipping short.
- **The folder settles `--no-video`, not the flag.** Two attempts got this wrong, and both look
  sufficient. A bare glob gathered a video build staged an hour earlier. Passing acceptable suffixes
  still failed, because the marker goes on the *declined* build. A product with no `video` feature
  never carries one, so the empty suffix has to stay acceptable. The empty suffix matches a video
  build too.

  What is actually true is whether the libraries are sitting there. So that is the question the
  script asks, and it decides what the generated README claims.
- **Versionless folders, versioned archives**, so `clean.sh --old` can never take one.

### Where a wallpaper pack goes

`km-display` reads a zip in the wallpaper folder as a folder of images, so a pack's natural home
looks like `assets/wallpapers/`. **One file reaching every desktop build for free is exactly what is
wrong with that.** A pack built once to look at travels into the Windows folder, the macOS bundle and
the tarball. It also reaches `dist/bin`, the installer and the APK, and nothing but the APK's size
note says so.

The default is `local/assets/wallpapers` — the checkout overlay, which km-app prefers over
`assets/` when run from a checkout, so `cargo run` still picks a pack up. `--zip-dest
./assets/wallpapers` is how a pack goes into a release, deliberately and by typing it, and
`check-assets.sh` then names it on every staging run.

## What a staging run says out loud

**Two mechanisms, and the split is the design.**

- **Cargo takes its own `--quiet`**, which drops `Compiling`/`Finished` and leaves every rustc
  diagnostic where it was. This is verified rather than assumed: a deliberate `E0308` came through a
  default-verbosity run in full, span and all.
- **Everything else goes through `dist_run`**, which captures to a temp file and replays the *whole*
  log to stderr on failure. `docker build`, `cargo deb` and `fetch-assets.sh` are its callers; none of
  them separates report from noise the way cargo does. It replays the whole log rather than a tail,
  because these commands are minutes long. Making somebody rerun one to find out why it failed is the
  cost this exists to avoid.

**`docker run` is deliberately not wrapped.** What the container prints *is* the report — the
`dpkg-deb --field` dump, the Depends assertions, the byte totals. Only cargo's stream inside it is a
log, so the quieting happens in there where the two can be told apart.

**`check-assets.sh` is the one thing that speaks up unasked**, and it earns it by saying what is about
to be copied. It refuses exactly one thing — two `.sf2` in `assets/soundfont/`, because the `.deb`'s
glob would ship both — and reports the rest. Inside `dist_stage_assets` its call is redirected to
stderr, because that function runs in a command substitution and its stdout *is* the file count.

Each build step prints `built in 2m14s` afterwards, from bash's own `SECONDS`. Elapsed time answers
silence, not a progress bar.

### `task run` does not wait, and a bare `&` will not do it

**Task's embedded shell is `mvdan/sh`, which waits for its background jobs**, so the launcher has to
be a process that forks and exits by itself:

| platform | launcher | why |
|---|---|---|
| Windows | `cmd /c start "" "$exe"` | `start` is a `cmd` builtin. The empty `""` is the window title, which `start` would otherwise take from the quoted path, leaving no program to run |
| Linux, macOS | `/bin/sh -c '"$@" … &' sh "$exe"` | a real `/bin/sh` does not wait for background jobs |
| macOS `.app` | `open -a "$PWD/$app"` | returns at once, and gives activation and a Dock icon |

`CONSOLE=1` blocks on Windows, and that is the rule rather than an exception. Asking for the twin that
prints means wanting to read what it prints.

### The Taskfile is an index, never a definition

**No task may carry a flag, a default, a feature list or an ordering the command it wraps does not.**
`task test` *is* `cargo km-test` and would be a bug if it were anything else.

The concrete test is the video feature list. `tools/setup/features.sh` and `.cargo/config.toml` spell it, and `check.sh`
asserts the two agree. `task build` reaches it through a `km-build` alias rather than spelling it.
That string has already been wrong in two places at once, and the Taskfile is not a third copy.
**`task` staying optional is what keeps that honest.**

**A wrapped script's positional argument becomes a variable named after the script's own name for
it.** That is why the two deploy tasks take `HOST=user@box`: `deploy.sh` and `appliance-boot.sh`
both hold it in `HOST`, and both spell their usage `[user@]host`. It is the one variable in the file
with no default and none available. The only value that would serve is the address of somebody's own
house, and no tracked file may carry one. So a missing host is a failure rather than a fallback, and
`_host` states it in the spelling that was typed. The scripts refuse it too, but they print their own
usage line, which names a command nobody ran.

Two consequences follow. **`task lint` does not forward `{{.CLI_ARGS}}`**, alone among the wrappers.
`km-lint` ends in `-- -D warnings`, so an appended argument would land after that `--` and reach
rustc. And **`dist:deb`, `dist:deb:tools` and `dist:tarball` are not gated on Linux.** All three
build entirely in Docker. Without them, the box this is mostly developed on could not reach the
`verify:*` tasks.

**The Taskfile carries one thing that the commands it wraps do not**, and it is a stated exception
rather than a lapse. A global `env:` sets `CMAKE_POLICY_VERSION_MINIMUM=3.5`. CMake 4 refuses to
configure SDL3_ttf's vendored FreeType, which still declares a 3.0 minimum. The failure is a
build-script panic naming neither CMake nor FreeType.

**The boundary is what makes it safe.** An environment variable that lets a third-party build system
configure at all is not a flag, a default, a feature list or an ordering. It changes nothing about
*what* the build produces. So `task build` and `cargo km-build` still yield the same artifact from
the same inputs.

**What it costs is stated plainly.** On a box with CMake 4, the two stop being interchangeable at the
point of *failure*: `task build` succeeds where a bare `cargo km-build` panics. So the export stays
documented in `CLAUDE.md` for the hand-typed case, which is the only one left that this does not
cover. `tools/platform/macos/app-bundle.sh` sets the same variable for the release build it drives.

## The Windows folder

```
karaokemachine.exe   karaokemachine-console.exe   README.txt
avcodec-61.dll  avformat-61.dll  avutil-59.dll  swresample-5.dll   ffmpeg-LICENSE.txt
assets/soundfont/…  assets/wallpapers/…
```

**Why a folder and not just the exe.** `Paths::discover_asset_dir` takes the directory beside the
executable if it has an `assets` child, and otherwise the working directory. `cargo run` from the
repository root is the *fallback* case. A bare copied exe silently loses its bank, its wallpapers and
its font. It comes up on a sine test tone over a gradient, with nothing saying why.

The script's first step is `fetch-assets.sh`. That is the point of it: the release step that puts the
SoundFont in the build rather than trusting whoever ran `cargo build` had fetched it.

**Deliberately not in the folder:**

- no SDL DLL: `build-from-source-static` links SDL3 and SDL3_ttf in, and `rusqlite` is `bundled`;
- no bundled font, since Windows has one at a known path and shipping one means shipping its license;
- no `remote-dev` folder, because the page is compiled in;
- no `km-pack.exe`, since a developer machine builds packages and the API installs them.

**The one thing it does not carry is the MSVC runtime.** `VCRUNTIME140.dll` is imported dynamically,
so the machine needs the VC++ redistributable — present on virtually every install and named in
`README.txt`. `-C target-feature=+crt-static` would remove even that, but it changes the CRT for the
vendored C in SDL3, FreeType and SQLite too.

**Two executables, and the trade that produced them was originally judged wrong.** The old argument
was that a console window is the price of keeping `--version`, `--show-paths` and `--set-password`.
That weighed a console against no command line, when the answer is *both*: a library with two
three-line binaries. `#![windows_subsystem]` is a property of a binary crate root.

The argument also mispriced the console. This is a fullscreen appliance under a television, so the
console was a black window sitting beside the lyrics all evening. The `README.txt` line telling
people to expect it was the tell: a feature does not need a warning.

The twin is a debugging tool rather than "the one to type", and only the words changed. A process
inherits its standard handles whatever the subsystem. So a GUI-subsystem `--show-paths` prints to an
inherited console or pipe perfectly well. What the twin buys at an interactive prompt is a shell that
*waits*. The README sentence is conditional (`If there is a …-console.exe beside it`) because that one heredoc
is copied into three carriers with different contents.

**It verifies rather than asserts.** The claim a video folder makes is that it is self-contained, so
the script ends by running the staged exe with `PATH` cut to `System32`. That is a real check. A video
build with its DLLs missing does not degrade: it dies at load time with `STATUS_DLL_NOT_FOUND`. The
message names a file rather than anything about video.

One trap: the script reads the version by running the freshly built exe, and at that moment the DLLs
have not been staged. So it puts `$FFMPEG_DIR/bin` on `PATH` for the duration.

The exe carries its own icon as a Windows resource. When that build script cannot run, it is a
**warning, not a failure**. Compiling a resource needs `rc.exe`, and a machine can have the MSVC
linker without it. Refusing to build the application over its icon would be the wrong trade.

`winresource` defaults `FileDescription` *and* `ProductName` to the crate name. Only a look at a built
binary shows that, so the scripts set both. `ProductName` is *KaraokeMachine* in all four scripts,
and `FileDescription` is each program's own name. Windows Firewall's allow-this-app dialog reads that
field, so it holds a name rather than the crate's `cargo` sentence. See
[`What the tool calls itself`](../decisions/foundations.md#what-the-tool-calls-itself) for every
place a name is spelled out.

### Video in a release build

**On by default in every staging script**, with `--no-video` asking for the smaller build. The cargo
feature keeps its own default, and the distinction is the point. Cargo's default answers *what must
somebody install to compile this at all*. A staging script's default answers *what does a person
receiving this folder get to play*. Getting the same answer to both was the mistake.

- **A missing ffmpeg stops the script before it builds anything**, naming both remedies. Falling back
  to a non-video build with a warning would reintroduce the exact failure this fixes, quieter. The
  exception is a run that selected only tools with no such feature. A default that fails on work it
  could not have applied to is a demand, not a default.
- **The marker goes on the declined build** (`-no-video`, `no-video/` for the `.deb`). Two builds of
  one version must not overwrite each other, and `--zip` must not clobber a zip already sent to
  somebody. `km-lyrics` and `km-wallpaper-pack` never carry it: naming a choice they were never
  offered would be a lie in a folder name.

**Four DLLs, not seven, and the list is derived rather than guessed.** `ffmpeg-next` is taken with
`codec`, `format` and `software-resampling`, so avdevice, avfilter and swscale are never linked. The
four that are are closed under dependency, read out of the import tables. Copying `bin/*.dll` would
add 29 MiB the program does not use and would stop being a statement about what it links.

**What each platform pays.** Windows: the loader searches the executable's own directory, so DLLs
beside the exe need no `PATH`, no installer and no variable. Linux: the `.deb`'s `Depends` names the
four and Debian supplies them, so nothing is staged. It is the one platform where video costs the
shipping story nothing. macOS: a Mach-O names each library by the absolute path it was linked at, so this is
the one that costs work.

## The Windows installer

One Inno Setup 6 installer carrying all eight products behind five component checkboxes; 75 MiB from
a 187 MiB payload.

**It gathers, it does not build.** The payload is `dist/bin/windows`, so nothing about features, DLLs,
console twins or READMEs is written down a second time. `dist/bin-console/windows` is not opened.

**One payload file is deliberately not installed: `README.txt`.** That is the exception to the
paragraph above, and it exists because gathering has a limit. The folder README is a *document about
the folder*, and an installed build is not one. It said the folder holds every executable this
platform can build, and it listed all eight programs. It said to remove it by deleting the folder,
because nothing was registered. Yet it was the `Read me first` Start Menu entry, beside an
uninstaller, a Start Menu group, a `PATH` entry and a file association.

`dist_installed_readme` writes the replacement. It lives in `common.sh` for the reason
`dist_ffmpeg_license_note` does. One copy prevents two setup programs from describing different ways
to remove the same product.

**The `.iss` names every file, and the driver reads `[Files]` back out.** Anything unaccounted for
**fails the build, by name**. The driver subtracts `*.WebView2` with a glob rather than one name,
because `km-remote.exe` links the same webview. It prints the count rather than dropping it in
silence.

**`[Components]` is read back out the same way**, for a reason the `[Files]` reconciliation would not
catch. The round trip installs "everything" by *naming* the components. So a list of them written out
in the driver stops agreeing the day somebody adds a product, and that has happened. An `assets`
component arrived with `km-admin`, and a verification install went on selecting the first four. The
check failed on the executable it had never asked for.

Both the summary line and the full install come from the `.iss`. A parse that finds no
`[Components]` fails the build rather than verifying nothing. The macOS driver already derives its
component list, its tool commands and its bundle table for exactly this reason. That same arrival
left three hand-written "six"s behind there.

**WebView2's default user data folder is beside the executable.** So a program that takes the default
puts its cache *inside* `{app}` after an install. That makes `{app}` a directory that has to stay
writable, and under `Program Files` no webview can be created at all. The symptom is a window that
never opens, rather than an error anybody could act on. Both programs name a per-user
cache directory through a `WebContext`;
`[UninstallDelete]` stays as belt-and-braces, guarding the next program that writes beside itself.

**Per-user, into `%LOCALAPPDATA%\Programs`, and no UAC prompt.** Everything configured beyond the
files is already per-user, and that justifies the scope. The `.kmbuild` association goes to
`HKCU\Software\Classes` by `register.rs`'s own decision, and the PATH entry to `HKCU\Environment`. A
machine-wide install would put files where every account can see them and configure them for one.

**The `.kmbuild` association is delegated, never duplicated:** `[Run]` calls the exe's own
`--register`. A GUI-subsystem program has nowhere to print a failure. So the exit code Inno collects
is the only thing that could notice one, and the entry carries no `nowait`.

**PATH is done in `[Code]`, both directions together.** `[Registry]` could add the entry. Taking one
back out of a value this installer did not create has no declarative form, so splitting the two would
invite drift. The code uses `RegWriteExpandStringValue`, because a real user's `Path` routinely
contains `%USERPROFILE%`. Rewriting it as a plain string would expand those permanently.

This is one of the two places where choosing Inno over NSIS actually paid. NSIS caps strings at 1024
in a stock build, so reading a long `Path` back truncates it.

**Verification is a round trip run at the end of every build:**

1. Install silently into a scratch directory with `/TASKS=""`, so it cannot touch this machine's
   PATH or associations.
2. Run `--version` on all eight with a bare PATH, and assert the **exit status**. That proves every
   imported DLL resolved from the install folder.
3. Uninstall, and assert the directory is gone.
4. Install `remote` alone, and assert it carries neither the assets nor the DLLs. Without that half,
   the component wiring could quietly stop saving the 140 MB it claims to.

Four details learned by getting them wrong:

- **A round trip that fails between the install and the uninstall leaves a registration behind.** An
  `EXIT` trap removes the scratch tree. A per-user install also writes itself under
  `HKCU\...\CurrentVersion\Uninstall`, and `unins000.exe` is the only thing that takes the entry back
  out. Deleting the folder deletes the uninstaller with it. What survives is a row in Settings >
  Installed apps naming a directory that is gone. It has no icon (`UninstallDisplayIcon` points into
  it), and an Uninstall button that cannot work.

  The first time it happened, somebody reported it as a Windows bug. The trap runs any
  `unins000.exe` it finds and waits for it, bounded, before removing anything. It lives in
  `tools/platform/windows/inno.sh` with the rest of the harness, so a second driver inherits the
  lesson rather than re-earning it.
- **Inno's `Setup.exe` can hand the work to a second process**, so its exit is not the end of the
  install. The loop waits for `unins000.exe`, which is written last. The uninstaller relaunches itself
  from a temp copy, so its assertion is on the directory disappearing, polled.
- **`ISCC.exe` with no arguments prints usage and exits non-zero**, so the version probe took the whole
  script down under `set -o pipefail`. The `|| true` on that pipeline is load-bearing.
- **`MSYS2_ARG_CONV_EXCL='*'` is required for every call into a Windows program.** Git Bash rewrites
  arguments that look like POSIX paths, so `/DPayload=...` arrives as `C:/Program Files/Git/DPayload=...`.

**A fourth `[Tasks]` entry writes the instrument-bank request.** The box starts ticked, and the
`machine` component gates it. It writes two lines of JSON from `{#Generated}` into
`%APPDATA%\karaokemachine\config`, with `onlyifdoesntexist uninsneveruninstall`. So a reinstall
cannot reset an attempt count, and the uninstaller cannot break its own closing promise.

The size and the license come from `tools/setup/soundfont-banks.sh` as `/D` defines. The driver
doubles the license's apostrophes, because it goes into a Pascal literal in `UpdateReadyMemo`, which
is where the terms are shown. The name and the size are deliberately *not* escaped. They also go into
a `[Tasks]` description, which is plain text, and neither can carry an apostrophe.

**The round trip asserts only the negative half**: that a `/TASKS=""` install leaves nothing in
`%APPDATA%`. The positive case is untestable there. Inno resolves `{userappdata}` through the shell
folders, which no environment variable redirects. So proving it would mean writing a real request
into the developer's own install, and their next start would fetch 262 MiB. The build asserts
instead that the generated file exists and names the row the table marks `recommended`.

Unsigned, so a recipient sees SmartScreen's *"Windows protected your PC"* and clicks through.

## The macOS installer

One `.pkg` carrying all eight products behind six visible ticks; 80 MiB from a 150 MiB payload.

**A seventh component package with no payload at all.** `soundfont` is built with `pkgbuild
--nopayload`: no `--root` and no `--install-location`. Its whole content is a postinstall that writes
the same instrument-bank request into the console user's `~/Library/Application Support`.

It is a second array in the driver rather than a seventh entry in `COMPONENTS`. Everything that array
drives asks a question about files, and a payload-free component cannot answer it. That covers
`claim()`, `install_location`, and the two checks reconciling what was staged against what was
archived.

Two things about that postinstall are worth knowing:

- **It runs as root and the request belongs to a person.** So it resolves the console user with
  `stat -f %Su /dev/console` and `dscl`. It chowns both the file and the folder it may have had to
  create. An install driven over SSH has root on the console and writes nothing. That is the honest
  answer when there is no home directory it could mean.
- **Nothing in it may fail the install.** `require-scripts="true"` is set, so every step falls out
  to `exit 0`, the same tolerance the builder's `--register` nudge has. "We could not write down
  which bank you wanted" must not become "the karaoke machine would not install". The build checks
  all four postinstalls with `sh -n` before it packages them. A heredoc that will not parse would
  otherwise fail on somebody else's Mac, with nothing on screen naming the line.

**Five of the six carry a payload, and they are five because of where they land.** `pkgbuild` takes
one `--root` and one `--install-location`, and `/Applications` and `/usr/local/karaokemachine` cannot
share one. So the count is forced rather than chosen. One of the five, `docs`, is hidden and always
selected, and it holds the READMEs and the uninstaller. So a tick cannot decline the one way to take
this off again.

`com.rrgmc.*` and not `com.karaokemachine.*`: package identifiers and bundle identifiers are
different namespaces that must not be conflated.

**All six bare executables go in `tools`, not symlinked out of their bundles.** `lib/` decides it.
Every one was staged with `@executable_path/lib`. So splitting them means either a second 15 MB copy
or a package that cannot stand alone.

### The claim table is the staging loop

The Windows driver reconciles by parsing `[Files]` back out of the `.iss`. There the description and
the payload are two things that can disagree. Here they are one: `claim()` maps a payload entry to a
component. **The staging loop is a walk driven by that function**, so no entry can be staged without
a claim. An unclaimed entry fails the build by name. Planting a stray file and watching it refuse
proves the check is not vacuous.

Three assertions sit behind it:

- Every component root is non-empty. A `case` arm matching nothing produces a smaller package and no
  error.
- The payload's file count equals the staged count plus three and minus the payload's own README.
  **The two adjustments stay separate, because letting them cancel is how the check passes while
  meaning nothing.**
- Each archived payload equals what was staged, read back with `pkgutil`, which asks the artifact
  rather than the intention.

**`.DS_Store` is this platform's `*.WebView2`.** The driver subtracts a top-level one by name and
refuses a *nested* one outright. `pkgbuild` drops every `.DS_Store` at any depth with its own default
filter; that is measured, not taken from the man page. Left alone, one inside a bundle would be
staged and dropped, and surface a screen later as a mismatch rather than as its cause. It does
**not** break the seal, because `.DS_Store` is in `codesign`'s own default exclusions and is never
sealed. So the mismatch is the whole of the reason.

### The second bundle, and the one thing that can be wrong with it

`KM Stream.app` is what `--stream` has instead of the Start Menu entry Windows gives it and the
desktop action Linux gives it. It is a bundle rather than a line in one, because a macOS manifest has
nowhere to put a launch argument. It holds a plist, an icon and a four-line script, and it copies
nothing. What it runs is the binary inside `Karaoke Machine.app` beside it.

- **`exec`, never a symlink**, and that is also why `/usr/local/bin/karaokemachine` is a shim. Rust's
  `current_exe()` on Apple is `_NSGetExecutablePath` with no realpath. So a machine that sees this
  bundle rather than the real one takes neither branch of `discover_asset_dir`, and comes up on a
  test tone. `exec` makes the kernel record the real path.
- **The path is resolved from the script's own location**, so the pair works unzipped into a Downloads
  folder as well as installed under `/Applications`.
- **Its `CFBundleIdentifier` is its own.** LaunchServices keys a bundle on that, and two bundles
  sharing one are one application to the Finder, the Dock and `open`.
- **So is its icon**, and that is the one thing it holds that the bundle beside it does not. They sit
  together in `/Applications` and in the Dock. One mark on both leaves two entries that differ only by
  the name under them. `Info.stream.plist` names `karaokemachine-stream`, and
  `dist_stage_macos_bundle` gets the matching `.icns`.
- **A row in `BUNDLE_STARTS` is what proves it reaches the machine.** It is the only thing about this
  that can be wrong. A relative path into a bundle renamed or moved is a launcher that starts nothing
  and says nothing. `--show-paths` returns before anything is opened, so the check asks the question
  without starting an encoder.

### Two `pkgbuild` traps that are silent when wrong

- **`BundleIsRelocatable` defaults to `true`.** With it on, Installer places a `.app` wherever an
  existing bundle with the same `CFBundleIdentifier` already is. So somebody who unzipped the app into
  `~/Downloads` gets *that* copy upgraded, and `/Applications` stays empty, with no error anywhere.
  Each bundle component gets a component plist, **derived with `pkgbuild --analyze` rather than
  committed**. So a bundle that changes shape cannot leave a stale one behind.
- **The check for it is not the obvious grep.** `<relocate/>` is present in *every* `PackageInfo`. It
  is the *list* of bundles that may move, and an empty one is the good case. So grepping for the word
  passes on a correct package and fails on one. `relocatable="false"` is the answer. The first version
  asserted the wrong one and stopped its own build.

### Symlinks, a shim, and the asymmetry that forces both

Two resolvers read the executable's path and they disagree:

- **dyld resolves `@executable_path` against the realpath**, so a symlink into the prefix finds
  `lib/`.
- **Rust's `current_exe()` on Apple does not.** It is `_NSGetExecutablePath` and nothing else, so
  through a symlink it returns the symlink; `discover_asset_dir` then matches neither branch and falls
  back to `$PWD/assets`. Measured: through a symlink `--show-paths` reports a non-existent directory;
  through a two-line `exec` shim it reports the bundle's `Contents/Resources/assets`. A test tone over
  a plain gradient is what the symlink would have shipped.

Of the seven, only the machine reads anything relative to its own executable, so only the machine
gets a shim. **The round trip asserts both halves**: that the shim finds the assets, *and* that a
symlink does not. An assertion on the shim alone would go on passing the day somebody canonicalises
`current_exe()` and makes the shim pointless.

**The postinstall creates `/usr/local/bin`, and the payload never archives it.** A payload naming it
puts its mode and owner in the bill of materials. Installer applies BOM directory entries to
directories that already exist. On an Intel Mac that directory is Homebrew's, so ours would quietly
break `brew`.

### Uninstalling, which macOS does not do for you

`uninstall.sh` removes the application bundles, the folder, the `/usr/local/bin` entries and the
receipts, and **nothing it identifies by name**.

- A bundle is ours if its `CFBundleIdentifier` starts `com.karaokemachine.`.
- A `/usr/local/bin` entry is ours if it resolves into `$PREFIX` or carries the shim's marker line.
- A receipt is ours if its id starts `com.rrgmc.karaokemachine.`.

A blanket `rm` of six names out of a directory this package did not create is exactly what an
uninstaller must not do. A list of names also cannot survive a rename, and that is the decision
`An uninstaller finds its own work, and never by name`.

**A test of that ownership check found a real bug.** `readlink` returns the target as written. So a
*relative* symlink compared against an absolute prefix reported every entry as somebody else's. The
postinstall happens to write absolute targets, so nothing would have shown it.

**The candidate set was the half that stayed hardcoded, and it cost three orphans.** The ownership
test was right from the start. What was wrong is that the uninstaller only ever asked it about names
a fixed list already knew. So a bundle or a symlink under a superseded name never reached it.

Two installs either side of a bundle rename left orphans behind:

- `KaraokeMachine Assets.app`, `KaraokeMachine Package Builder.app` and `KaraokeMachine Remote.app`
  in `/Applications`;
- dangling `km-assets` and `wallpaper-pack` links in `/usr/local/bin`;
- an `assets` receipt beside the live `admin` one.

The uninstaller scans `/Applications`, `$BINDIR` and `pkgutil --pkgs` and asks the existing test about
every entry. That finds all six without being told any of them exists.

It was right from the first build and **unfindable**. Only two Installer panes named it, and not even
the README beside it did. `Uninstall KaraokeMachine.command` is the tarball's answer ported. `.command`
is the extension the Finder hands to Terminal. The file is a wrapper, so `uninstall.sh` stays the
only thing that removes anything.

Four things in it are load-bearing:

- `cd "$(dirname "$0")"` first, because Terminal starts in the home directory;
- the dry run **before** the password prompt;
- a tty check, with the confirmation defaulting to no;
- `exec sudo`, so the prompt lands in the window showing the list.

The round trip runs it with stdin closed. It asserts that the wrapper removes nothing **and names the
direct command**. That second assertion is load-bearing, and not obviously so. Without the tty check,
the `read` gets EOF and the answer is empty. The defaults-to-no branch then prints *"Nothing was
removed"* anyway, so the first assertion passes on a broken wrapper.

**The "your songs are left alone" message is written once and said twice**, from
`pkg/data-locations.txt`. macOS has no uninstaller UI, so the sentence must appear in both the
installer's closing pane and the uninstaller's output. Running the generated script is what caught the
substitution bug, and its shape is worth knowing. The template's own header comment also mentioned
the marker, so a plain replacement dropped ten lines of prose into a `#` comment. The result is still
*valid shell*, so `sh -n` passes it. The build now asserts each marker occurs exactly once.

**The panes are sniffed, not declared.** `mime-type="text/html"` does not settle it. Installer sniffs
the file's *data*, and a file beginning with an HTML comment fails that sniff and renders as plain
text. Both panes begin with `<!DOCTYPE html>` before anything else, comments included, and the driver
asserts it on every build.

### Signing

Unset, signing is ad-hoc. `KM_SIGN_IDENTITY` set, `dist_codesign` signs with it — `--options
runtime`, `--timestamp` — and every site that signs calls it. **One function rather than four spelled
`codesign` lines.** A bundle may have ad-hoc dylibs and a Developer ID executable. Notarization
refuses that bundle while naming neither file.

`KM_SIGN_INSTALLER_IDENTITY` is separate because it is a separate certificate type. The half-signed
combination is refused up front: Gatekeeper judges the archive, so signed bundles inside an unsigned
`.pkg` buy a recipient nothing.

**`installer.sh --notarize` is the one path where all three have committed defaults**, so `task
dist:setup:notarized` needs nothing set. The script applies them inside the `NOTARIZE` gate rather
than at the top of the file. `dist_signing` keys off `KM_SIGN_IDENTITY` being non-empty, so a default
outside that gate would make every build sign.

`KM_SIGN_IDENTITY` is the one of the three that is **exported**. `tools/dist/bin.sh` runs as a child
process, and `dist_codesign` signs there. So an unexported assignment would stage an ad-hoc payload
under a Developer ID wrapper. That is the half-signed shape above, and only the round trip after a
full staging run finds it.

Four things learned by running it:

- **`grep -q` under `set -o pipefail` reports the opposite of what it found.** It exits at the first
  match, SIGPIPEs whatever is upstream, and `pipefail` makes the status 141. The same flaw was in
  `dist_verify_macho_portable`'s absolute-path check, where it fails in the *dangerous* direction. A
  real absolute load path makes `grep` match and exit, and the `if` reads 141 as *no match*. Both
  capture output first and match afterwards.
- **`-R` needs its value quoted.** `subject.OU = TEAMID` is a syntax error; `= "TEAMID"` is not.
- **`--options runtime` must be applied at every signing call, not to the bundle alone.** Sealing does
  not add the hardened runtime to code already signed inside it. `--verify --deep --strict` does not
  check that it is there. So a partial application passes every check here and fails notarization.
- **A locked keychain makes signing block on an unlock prompt with no output**, because `dist_run`
  buffers and replays only on failure. `productbuild` runs outside `dist_run` when signing.

**Notarizing rather than only signing is what removes the dialog**, in the platform's own words.
Signed-but-not-notarized is `rejected / source=Unnotarized Developer ID`. The same archive notarized
and stapled is `accepted`. `stapler validate` passes on a quarantined copy, and that says the ticket
travels in the file rather than being fetched.

**Verification needs no password.** A system-domain `.pkg` installs to `/` and needs root. So the
default round trip expands the archive and inspects what would land. It then builds the installed
layout in a temporary directory and runs it. `pkgutil --expand-full` is **undocumented**, in
neither the man page nor `--help`, so the script probes it rather than trusting it, with `--expand`
plus `cpio` behind it. `--install` is the opt-in real thing.

## The remote's own installers

A second carrier on each desktop platform, holding `km-remote` and nothing else; 5 MiB from a 20 MiB
payload on Windows. The product decision is `A setup program for the remote alone`; this is how it is
built.

**The payload is `dist/km-remote/<platform>/km-remote-<version>-<triple>`**, which
`tools/dist/cmd.sh` already stages: five files, two seconds. The alternative, `dist/bin/<platform>`,
is every product and a six-minute run. The folder is *found* rather than named. `dist_staged_dir`
takes a version, and the version comes out of the binary inside the folder. So the driver globs for
one match and refuses two, the shape `release.sh`'s `one_match` uses.

The driver refuses a `-no-desktop` payload by looking for what `cmd.sh` stages only alongside a
window: the console twin on Windows, the `.app` on macOS. A setup program installing a remote that
cannot open one is not what it promises. That is the same refusal the all-in-one makes about a
payload with no ffmpeg in it.

**What the two drivers share is in `tools/platform/windows/inno.sh`:**

- the compiler search and its Inno-6 assertion;
- the two `sed` programs that read `[Files]` and `[Components]` back out of a script;
- the payload reconciliation;
- the install-run-uninstall harness.

The harness is the piece that had to move; see the orphaned-registration bullet above.
`tools/platform/windows/webview2.iss` is the same bargain for the `[Code]` section. It holds the
three-registry-view runtime probe and the bootstrapper download. Both scripts `#include` it, and the
two sentences that differ come in as defines. Each script keeps its own `NeedsWebView2`, because
Pascal Script resolves in order and the question genuinely differs. One asks which components were
ticked, and the other asks nothing.

**Two `.iss` files rather than one compiled twice.** The drivers read the script with `sed`, which
knows nothing about the preprocessor. So a file whose `[Files]` and `[Components]` depended on a
define would hand each driver the other's lines. That reconciliation is the only thing standing
between a carrier and a silently dropped file. The two also disagree in nine sections, which is not a
variant.

**The coverage check is the same rule with a different exclusion.** The all-in-one subtracts the
folder `README.txt` and installs `README-*.txt`. The remote-only carrier subtracts
`km-remote-console.exe` and installs the folder README *as* `README-km-remote.txt`. That document is
the remote's own, and it is right wherever somebody reads it. The short one beside it is
`dist_installed_readme windows km-remote`. That is a product argument rather than a third platform,
so four setup programs cannot come to describe removing the same two products four ways.

**Two assertions with no sibling in the all-in-one.** The build asserts the ffmpeg list *absent* from
the script rather than present in the payload. The remote links none of it. One careless `[Files]`
line would turn a 5 MiB download into a 40 MiB one with nobody deciding to.

The build also compares `AppId` and `AppName` against `installer.iss`, and it refuses when either
matches. Inno decides upgrade-versus-second-copy by `AppId`. So the copy-paste that shares one is the
only realistic way a remote install comes to remove the karaoke machine. The install folder and the
Start Menu group both follow `AppName` in each script, so two comparisons cover four values.

**`dist/setup/windows/km-remote-generated` and not `generated`.** The all-in-one clears its own
generated directory on every run, so one shared directory would mean whichever installer built last
owned the other's README.

**The round trip is the all-in-one's, shorter and mostly negative:**

1. Install, and run `--version` on a bare `PATH`.
2. Assert the absences that define the carrier. There is no console twin, no other product and no
   `assets/`. There are none of the four DLLs, and no ffmpeg licence beside no ffmpeg.
3. Check both READMEs, then the planted `*.WebView2` folder.
4. Uninstall, and assert the directory is gone.

### The macOS half

`tools/platform/macos/pkg.sh` is what `inno.sh` is one platform over. The same question drives the
selection: which of these, copied and drifted, produces something that *looks* fine? It holds:

- `pkg_resolve_signing`: two certificates, three states, every refusal, and the `export` that makes
  the staging subprocess sign the same way;
- `pkg_component_plist`;
- the template fills, with their occurs-exactly-once guard;
- the two pane renderers and the doctype assertion;
- `pkg_expand`'s probe of the undocumented `--expand-full`;
- `pkg_notarize`.

**Two component packages, because `pkgbuild` takes one `--root` and one `--install-location`**:
`com.rrgmc.km-remote.app` into `/Applications`, `com.rrgmc.km-remote.docs` into
`/usr/local/km-remote`. The count is forced rather than chosen, and the second is hidden and always
on for the reason the all-in-one's `docs` is.

**The receipt namespace is the coexistence mechanism, and it is `com.rrgmc.km-remote`.** The
all-in-one's uninstaller forgets receipts by the `com.rrgmc.karaokemachine.` prefix. A component here
that borrowed that namespace would lose its receipt to a package that never wrote it. The round trip
greps the expanded archive for that prefix and fails on a hit.

**The bundle identifier is the opposite answer to the same question.** `com.karaokemachine.remote`
stays, because it names the application and two bundles may not share one — so both packages place
the same `/Applications/KM Remote.app`. Each uninstaller therefore asks whether the other's receipt
is present before taking it, and says so on the line where it does not.

**The payload is two paths, because that is the shape `cmd.sh` stages.** `KM Remote.app` sits beside
the versioned folder rather than inside it. A product's platform folder holds exactly one bundle, its
own, and `dist_stage_macos_bundle` takes a second one as the fossil of a rename. So the application
comes from one path and the licence texts from the other. The absent bundle is what a `-no-desktop`
staging looks like here.

**The claim table is a `case` with one arm and two named exclusions.** The two licence texts are all
the driver asks the folder for. The bare `km-remote` and `README.txt`, the terminal form and the
folder document, are the exclusions. The file-count reconciliation subtracts the skipped entries on
one side. It adds the bundle's own files and the three generated ones on the other, each kept
separate so they cannot cancel.

**Each half can be verified only on its own platform.** `pkgbuild`, `productbuild`, `pkgutil`,
`codesign`, `spctl` and notarization are all macOS. Its `--install` arm needs a password on the Mac it
runs on. The Windows half installs and uninstalls on the developer's own account there. That
round trip is the only thing standing between a payload rule changing and a package shipping short.

**Coexistence is the one thing neither round trip covers**, because it needs both packages on one
Mac at once. The check runs each uninstaller first in turn. The one that runs first keeps
`/Applications/KM Remote.app` and says so, because the other carrier's receipt still claims it. The
one that runs second takes it.

## The macOS bundles

`Karaoke Machine.app`, `KM Stream.app`, `KM Package Builder.app`, `KM Simple Package.app`,
`KM Remote.app` and `KM Admin.app`.

**Why a bundle rather than the bare binary.** A bare Mach-O is a terminal program. It has no icon in
the Dock or Finder, it is not double-clickable, and it has no `Info.plist`. So nothing names the
window, and there is no `CFBundleIconFile` to draw.

**One helper, four callers.** `dist_stage_macos_bundle` and `dist_seal_macos_bundle` in `common.sh`.
The contract worth remembering is the icon's. **The `.icns` is copied under its own basename, and the
plist's `CFBundleIconFile` must equal that name without the extension.** A mismatch is an error
nowhere: it draws a generic icon and says nothing.

**Every bundle is sealed, not only a video one.** It costs milliseconds, and it buys
`dist_verify_macho_portable` running on that build too. That check would catch a stray absolute load
path in a bundle that carries no ffmpeg but still links SDL.

**Staged from the pristine build output, never from the folder's copy.** That copy already has its
load commands rewritten to `@executable_path/lib`. Handing it over a second time would send it hunting
for `@rpath/...` dependencies and finding none. Two copies of one build, two rpaths, one
`cargo build`.

The plists differ, and each difference is a decision:

- **The remote declares no document type.** It opens no document and takes no positional argument. The
  builder's plist argues that declaring a type you cannot open is worse than declaring none; this is
  that argument in the negative. Hence `--no-desktop` takes the bundle **and** the window together. A
  document double-clicked arrives as an Apple Event. So a bundle around a build with no event loop
  would get a corpus and have nowhere to put it.
- **The builder declares four `NS*FolderUsageDescription` keys**, because it walks a folder of
  somebody's songs. The remote declares none, because it reads nothing the user chose.
- **Three of the four declare `NSLocalNetworkUsageDescription`**: the builder, the remote and the
  assets tool. Since macOS 14, browsing mDNS and then talking to a LAN address raises a prompt against
  the *bundle*. The same executable run from a terminal inherits the terminal's grant and is never
  asked. **The machine is the one without it, and that is correct.** It advertises and never browses,
  so nothing in it constructs a `Watcher`.
- **`LSApplicationCategoryType` is the one key all four declare**, and only the value differs. It is
  Music for the machine and the remote, and Utilities for the builder and the assets tool. It is advisory,
  because nothing at run time reads it. So `installer.sh` catches a manifest that omits it, or names a
  category macOS would silently ignore, rather than a build log. It reads every `Info*.plist` by glob,
  in the same pass that checks `LSMinimumSystemVersion` against the Distribution. The reasoning is in
  `What a macOS bundle says it is for`.

**A bundle has a TCC identity of its own and a folder does not**, and only a Mac shows that. The
builder's first launch stops on a Desktop-access prompt *before* it serves a page. So the window is
white until somebody answers. The keys do not remove the prompt and are not meant to: they make it
say why.

**A newly built unsigned bundle is also slow the first time it is opened**, while Gatekeeper assesses
it. It is slow enough that a first launch looks like a hang. The socket is bound throughout, so the
symptom is a connection accepted and never answered.

### Making the macOS video builds portable

**The closure is walked, not listed**, and that mattered more than expected. `km-video` links four
ffmpeg libraries. Against Homebrew's ffmpeg, those four pull in nine more: x264, x265, SVT-AV1,
libvpx, dav1d, LAME, Opus and OpenSSL's two, for **13 dylibs and 33 MB**. Against the pinned LGPL
build they pull in nothing at all, for **4 dylibs and 15 MB**.

The same code produces both, because it starts at the binary and follows every non-system load
command. So *changing which ffmpeg is built* answers the license question, rather than editing
anything here.

Four things are load-bearing, three learned the hard way:

- **Key by the install-name basename, not the filename on disk.** The load command says
  `libavcodec.62.dylib`; the file in the Cellar is `libavcodec.62.28.101.dylib`. The load command is
  what has to be satisfied at run time.
- **Every file touched must be re-signed.** `install_name_tool` invalidates the ad-hoc signature, and on
  Apple silicon dyld then refuses to load it. A build that skipped this would stage perfectly and fail
  at *launch*, on somebody else's Mac. The signature is stripped before rewriting rather than left to be
  invalidated, which also silences forty warnings.
- **`Contents/Frameworks` holds code, and `codesign` enforces it.** The license texts went there first
  and `codesign` refused to seal the bundle. They live in `Contents/Resources/ffmpeg/` instead.
- **Sign the bundle last.** Signing a Mach-O at `Contents/MacOS` is a *bundle* signature, so anything
  staged afterwards invalidates it. That is how the previous point came to light.

**The license question this raised.** Nothing was redistributed before; now something is. Homebrew's
ffmpeg is `--enable-gpl` with x264 and x265. So the first version shipped a GPL bundle while Windows
shipped LGPL. And **the application cannot call either encoder**, because every codec context in
`km-video` is a `Decoder`. So it was 25 MB of dead GPL weight.

`fetch-ffmpeg.sh` builds a pinned LGPL ffmpeg from source on macOS instead. It builds from source
rather than downloading, because **nothing publishes what this needs**. BtbN has no macOS target,
evermeet ships a static GPL command, and ffmpeg.org is source only.

## The Debian package

`.deb` for Debian 13 amd64, built in a container. **`FROM debian:13-slim` is the compatibility floor**
and the only thing to change to retarget. It is a container and not the WSL Ubuntu on the same box,
because that Ubuntu is newer than Debian stable. A package built there installs on trixie and then
refuses to start. "The .deb does not run on Debian" is the one way this deliverable can be wrong while
looking finished.

**The layout is `/opt`, and that is a decision.** Split it the conventional FHS way, with the binary
in `/usr/bin` and the assets in `/usr/share/karaokemachine`. Then *neither* branch of
`discover_asset_dir` finds them, and the machine comes up on its sine test tone, silently. So the
package keeps them together, with a `/usr/bin` symlink from the postinst. The symlink costs nothing,
because `current_exe` reads `/proc/self/exe`, which resolves *through* it. A C probe verified that
before the layout was chosen.

The desktop entry and the `hicolor` icons are what make it an installed application rather than a
binary in `/opt`. `Icon=karaokemachine` is an icon-theme *lookup*, not a path, so each size goes to
its own directory. There is no cache-refresh call in `postinst`, because the two relevant packages
ship dpkg triggers on exactly those directories.

**`depends` is written out, and `$auto` is the thing it replaces.** `dpkg-shlibdeps` derives only
`libasound2t64, libc6` from this binary. SDL3 is linked statically *and* opens X11, Wayland and GL
with `dlopen`, so none of them appears in the dynamic section for it to see. It would also resolve
the ffmpeg libraries the package *carries* against the build image's `-dev` packages. That puts back
the dependency the bundling removes. So the list is written out, and `$auto` is gone from the default
build.

`deb.sh --system-ffmpeg` is the variant that still asks for it. It links the distribution's ffmpeg
and takes a derived `libavcodec61` with it.

**What replaces it is a `DT_NEEDED` allowlist over the shipped binary**, in `deb-in-container.sh`:
an X, Wayland, VA-API, VDPAU or OpenCL library there fails the build. That is the requirement in
[`An appliance install puts no X library on the box`](../decisions/distribution.md#an-appliance-install-puts-no-x-library-on-the-box)
stated as a check, and it is what notices a new link-time dependency now that nothing derives one.

**Where ffmpeg shows itself moved with it.** A package carrying its own libraries Depends on none. So
`Depends` says nothing about whether video reached the binary, and `deb-in-container.sh` reads the
package's *contents* instead. It asserts `opt/karaokemachine/lib/libavcodec.so.*` and
`libopenh264.so.*` for a bundled build, and the `Depends` field for a system one. Each assertion runs
in both directions, so a run that took the wrong variant cannot pass the wrong test.

**Every one of those matches captures its producer's output and searches it afterwards**, for the
reason `tools/dist/common.sh` gives. Under `pipefail`, a `grep -q` that matches kills `dpkg-deb` with
SIGPIPE, and the pipeline's 141 makes the `if` read a match as no match. Over a seventy-megabyte
package the producer loses that race every time. The direction is the danger: each check would wave
through exactly what it exists to stop.

**What container verification proves, and what it cannot.** It proves the layout, the symlink, that apt
can satisfy the derived `Depends`, and that asset discovery resolves through the symlink to the bundled
bank. It **cannot** prove the bank is *opened*: a container has no sound device, and the run says so
honestly rather than pretending.

### A second package, from a manifest that is not the machine's

`deb.sh --tools` builds `karaokemachine-tools` — `km-package-builder`, `km-package-simple`,
`km-remote` and `km-admin` in one package the machine Recommends, per
[`The desktop tools are a package of their own`](../decisions/distribution.md#the-desktop-tools-are-a-package-of-their-own-which-the-machine-recommends).
It shares `deb-in-container.sh` with the machine because it shares everything around the build: the
image, the target volume, the checkout guard and the report. What differs is which manifest cargo-deb
is pointed at, `[package.metadata.deb]` living on `km-package-builder` and naming the package for
what it holds.

**km-admin comes from the second cargo workspace**, so the tools build is two `cargo build` calls
under one `CARGO_TARGET_DIR`. That lets one asset list name all four under `target/release`.

**It is a release carrier, so `task dist:deb:tools` is a task of its own rather than
`dist:deb -- --tools`.** `dist:app:linux` names it beside the other two, and `tools/dist/release.sh`
prints it to somebody about to type it. The package goes to `dist/karaokemachine-tools/linux/`,
a folder rather than a filename marker, both packages carrying a name of their own.

**`$ORIGIN/../lib` is the whole of why they live in `/opt`.** The builder links the four ffmpeg
libraries and carries none; the machine's package has them at `/opt/karaokemachine/lib`, and an
rpath from `/opt/karaokemachine/tools/` reaches them. `/usr/bin` could not, so the names there are
symlinks that `postinst` writes. `postrm` reads each one's target before removing it. So an upgrade
cannot take away a link of the same name that somebody else owns.

**What the verifier proves is that the rpath resolves**, the one thing nobody can read off the
package. `verify-deb.sh --tools` installs both `.deb`s in a clean container and canonicalises what
`ldd` reports for `libavcodec`. `ldd` prints the rpath as written, `tools/../lib/…`. So comparing the
text rather than the file makes a check fail on a package that is correct.

### The warm cache lies when there is more than one checkout

One volume, `karaokemachine-deb-build`, holds `/build/target` for `deb.sh`, `tarball.sh` and
`check.sh` in every checkout and every worktree. One volume rather than one apiece is what makes a new
worktree cheap. SDL3, SDL3_ttf and the bundled SQLite are minutes of C, and a shared volume has them
compiled already.

The price is that cargo cannot tell two checkouts apart. Each bind-mounts its own root at `/src`, and
fingerprints record that path. So a warm cache will hand a run artifacts compiled from different
source at the same location. In a check, that is a baffling compiler error, and the answer is to
clean. In a release artifact, it is a package built quietly from another checkout's code, and
`deploy.sh` installs it on the appliance.

So the release paths clean when the volume's stamp names somebody else. They clean the workspace's
own crates only, and leave the third-party half that costs
the minutes.

**Incremental compilation is off in the image**, because the volume outlives the containers and they
build once each. Incremental pays for itself on the second build of an edited crate, and there is no
second build here. Its per-crate caches accumulate under the debug profile across checkouts and
toolchains: disk written on every run and read on none. The release profile the carriers are built
from has it off regardless, so this reaches `check.sh` alone.

## The portable folder

The second Linux carrier, beside the `.deb` and not instead of it. Everything at the top level rather
than under `bin/`, for the same `discover_asset_dir` reason the Windows folder has that shape.

### The ffmpeg is built here, and that was not the plan

Calling Linux the platform where video costs the shipping story nothing is about *naming*, and it does
not survive contact with a folder. Two independent reasons, either sufficient:

- **License.** Debian's own copyright file says the default packages use GPL-licensed files, so the
  binaries are GPL-2+. This workspace is `MIT OR Apache-2.0`, and Apache-2.0 is not GPL-2-compatible.
  A `.deb` escapes it because naming a dependency is not distributing it. A tarball does not.
- **Size, measured.** `ldd` over Debian's `libavcodec.so.61`: **93 shared libraries, 97 MiB**. That
  is x264, x265, rav1e and libjxl, and behind them harfbuzz, fontconfig, pango, cairo and glib,
  because those codecs want text and images.

So the build makes ffmpeg **7.1.5**, exactly what trixie ships, and both Linux carriers decode through
the same code. The configure line matters less than the one flag that does the work:

**`--disable-autodetect` is the rule: every decoder ffmpeg implements itself, none that needs a
third-party library.** It is deliberately not a hand-picked codec list. A list needs another look
every time somebody auditions a file it did not anticipate, whereas this is a property with a reason.

**The rule is about decoders, and there is one encoder outside it.** A streaming machine has to
produce H.264, and ffmpeg implements no encoder for it. So `--enable-libopenh264` asks for the one an
LGPL configure line may have. What that costs the tarball is a fifth library in `lib/`. The build
copies openh264 into the prefix beside the four libraries that link it, under BSD-2 with its text in
`LICENSES/`. So the folder still asks the user's machine for nothing.

What comes out links `libc`, `libm`, `libz` and `libstdc++`. It links the last of those because
openh264 is C++, where everything else here is C. The folder does not carry libstdc++. A bundled one
older than the system's breaks any C++ loaded after it, and it is on every distribution the tarball
claims.

**The pin lives in `tools/setup/ffmpeg-pin.sh`**, and both the macOS and Linux builds source it. They
reached the same version, checksum and load-bearing flag independently, days apart. That is good
evidence the conclusion is right, and poor practice to leave as two files. They had already drifted
by `--enable-zlib` before anybody compared them. So a `.mov` with a deflated header played out of the
macOS bundle and not out of the Linux tarball. **What is shared is the definition, not the
procedure.**

Two details worth not rediscovering:

- **`nasm` is not optional.** Without it, configure quietly builds a C-only decoder. Nobody sees that
  failure at build time. It is a 1080p file dropping frames on the appliance, found much later and
  blamed on something else.
- **The binary is compiled against this prefix**, not Debian's headers. Building against one 7.1.5 and
  shipping another would *probably* work. The failure mode of "probably" is an undefined symbol at
  load time on somebody else's machine.

### RPATH, twice

`patchelf --set-rpath '$ORIGIN/lib'` on the binary, and `--set-rpath '$ORIGIN'` on each bundled
library. **The second is easy to miss: an object's own dependencies do not inherit RUNPATH, which is
what a modern linker emits.** The binary finding `lib/` says nothing about `libavformat` finding
`libavcodec` beside it. The symptom is a tarball that works on the build machine and fails everywhere
else.

The build runs `patchelf` on the staged file rather than putting `-C link-arg=-Wl,-rpath` in
`RUSTFLAGS`, for a cache reason. `RUSTFLAGS` is part of cargo's fingerprint. So setting it would make
the tarball and `.deb` builds invalidate each other in the shared volume.

Libraries are copied **under their sonames as real files**, not a versioned file plus a symlink
chain. `DT_NEEDED` records the soname, and nothing looks for the longer name.

### What is asserted before it ships

- It starts the staged binary under `env -i`. That is stronger than a stripped `PATH`, because it
  also drops `LD_LIBRARY_PATH`. That variable is exactly the one that would make a broken RPATH look
  fine on the build machine.
- A video build must resolve `libavcodec` **out of `./lib`**; a `--no-video` build must have staged no
  `lib/` at all.
- No bundled library may link `libx264`, `libx265`, `librav1e` or `libjxl`. This checks the license
  claim rather than trusting it. The plausible way it breaks is somebody adding a `-dev` package to the
  image. After that, `--disable-autodetect` is the only thing standing between this and a GPL
  dependency.
- The bundled `libavcodec` must carry `libopenh264`, asserted the same way. Without the flag,
  configure leaves a folder that decodes and plays perfectly and cannot stream. Whoever first runs
  `--stream` out of it would find that.

### The verifiers, and why only one may prewarm

`verify-tarball.sh` installs only the runtime libraries `README.txt` tells a user to install. So a
pass means that prose list is correct *and* complete, the part of a tarball no `dpkg` enforces.
`--image` is what makes the Fedora and Arch lines more than a guess.

Images are prewarmed and tagged by a hash of the *generated* Dockerfile text. Hashing the generated
text rather than the inputs separately is the point. It covers the base ref, the package list and the
install command by construction, so nothing can be left out of the hash.

**No package name is written anywhere in `verify-image.sh`**, and that is deliberate. A prewarmed
image is exactly where somebody later adds "just one more package" to turn a red verification green.

**`verify-deb.sh` cannot be treated the same way**, and this is the sharp part. It asserts two things:
that apt can satisfy the `Depends` from the archive, and that the `Depends` list is **complete**. The
second holds only because the image starts with nothing, so an omitted library makes `--version` fail.
Property two does not survive prewarming the closure. An omitted `Depends` is precisely what `$auto`
gets wrong, since it cannot see what SDL `dlopen`s. **So the closure is never prewarmed: not as a
mode, not behind a flag, not "just for iteration".**

A step called `verify` that cannot detect a missing dependency is worse than no step. It consumes the
attention a real check would have received.

What it caches instead installs nothing: the apt index, and the downloaded `.deb` files in a named
volume. One trap: official Debian images ship `docker-clean`. Its `DPkg::Post-Invoke` deletes the
archives after every install, so the cache volume would be emptied on the way out.

**Prewarming costs freshness, and the verifier makes that visible rather than hiding it.** It prints
the image's age on every run. `--refresh` rebuilds it, and `--no-prewarm` goes back to installing at
run time. `--no-prewarm` earns its keep for a second reason: a branch nothing ever takes is a branch
that has quietly stopped working.

`prewarm.sh` builds all of it up front, so a cold machine pays once and deliberately. **It must never
become a required step**, and it is not: every consumer keeps its own on-demand guard and calls the
same helper. It warms images only. It leaves the cargo cache cold on purpose. SDL3 and the bundled
SQLite compiling into `/build/target` would turn this into a twenty-minute command nobody runs.

### What this carrier deliberately does not do

No systemd unit, no `karaoke` user, no DRM master, no `/usr/bin` symlink. The appliance is the
package's job. `install.sh` does the one thing a folder can honestly do. It writes a desktop entry for
one user, with `Exec` rewritten to wherever they unpacked it, and `--uninstall` undoes it. A tarball that wrote
into `/usr` would be a package manager keeping no records.
