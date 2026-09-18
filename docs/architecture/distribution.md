# Distribution — the carriers

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## Where releases go

```
dist/<app>/<platform>/<the staged thing>
```

**App first, then platform.** A release is a thing you hand to somebody, so the useful grouping is
every build of one product together rather than every Windows build of everything.

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
that directory. Elsewhere the question is *whether* there is one — on macOS a perfectly good Homebrew
install has no DLLs in it, and on Linux `pkg-config` finds a distribution ffmpeg while `--print-dir`
has nothing to print. The README each folder carries had the same fault and it mattered more, because
a folder outlives the run that made it: `video_runtime_note` writes whichever of the two is true, and
off Windows names the shared libraries to install.

Two scripts deliberately keep their own copy of something and say so in place:
`tools/port/machine/android/assets.sh` repeats the asset loop because it copies into the APK's asset
*root*, and `tools/platform/linux/check.sh` keeps `host_path` because it stages nothing.

### The two slots that are not products

```
dist/bin/<platform>/           the windowed form of anything that has one
dist/bin-console/<platform>/   the console form; every single-form tool in both
```

- **It gathers; it does not build.** It runs the same staging scripts `task dist` runs and copies
  what they produced, so nothing about features, DLLs or READMEs is written down twice.
- **Its knowledge is three rules about shape, not a table.** A `*.app` is a windowed form and the
  bare executable staged beside it is therefore the console one; a file `<x>-console<EXT>` is a
  console form and its `<x>` sibling is therefore the windowed one; everything else is single-form
  and goes in both. **The first two are one rule in two spellings** — Windows names the pair as two
  files, macOS as a bundle beside the executable it wraps — and rule 1 had only its first half until
  it was noticed that `dist/bin/macos` held `km-remote` and `KM Remote.app` side by side.
  `has_bundle` asks the staging directory rather than a list of products, which is what keeps
  `--no-desktop` right for free: that flag stages no bundle, so the bare executable is single-form
  again and goes to both. `is_program` decides by **name, not the mode bit** — on
  Windows every file reports executable to a Git Bash `-x`, and off Windows the tarball's `install.sh`
  is 0755, so a mode-based rule would copy it in as a product and then *run it* during the checks.
- **Anything matching no rule is named, not dropped**, so the rules can stay narrow.
  `km-package-builder.exe.WebView2/` is the case that proves it exists.
- **A rule about shape cannot tell a bundle from its own fossil.** `gather_bundles` globs `*.app` in
  a product's staging folder, and a rename leaves the old name sitting beside the new one: nothing
  cleared it, because `dist_clear` empties the bundle it is about to *write* and cannot see a
  sibling, and `tools/dist/clean.sh --old` matches a name followed by a version that a bundle does
  not carry. So the glob gathered both, and the setup program refused the payload —
  `KaraokeMachine Package Builder.app, which no component claims` — which is the right answer to the
  wrong question. It also surfaced nowhere near its cause: the rename that made it moved three
  bundles on a machine with no `dist/` to fossilize, and the failure arrived a day later on one that
  had staged before it. **Fixed where the fossil is made rather than where it is found** —
  `dist_stage_macos_bundle` takes any other `.app` beside the one it is staging, unless the product
  has declared it in `DIST_MACOS_ALSO_STAGES`. The machine is the one product that declares anything:
  it stages a second bundle so that `--stream` has a launcher, and each of the two would otherwise
  sweep the other as a fossil. An array rather than a list, because two of the bundle names have
  spaces in them. Everything not declared is still a fossil and still goes — derived, so no dead name
  is written down anywhere and the next rename needs no edit. What it does not reach is a whole
  product folder for a program that no longer
  exists — `dist/km-assets/`, `dist/wallpaper-pack/` — because nothing stages into one to notice;
  that stays `--all` or a hand removal, argued above `sweep` in `clean.sh`.
- **Completing rule 1 moved files, and a consumer that read one folder lost them.** The macOS setup
  program took its whole payload from `dist/bin/macos`, which was every product's every form until
  the bundled three stopped leaving a bare executable there. It kept building: all four `.app`
  bundles were correct, so the package looked right and installed **four commands into
  `/usr/local/bin` instead of seven**. The lesson is not about macOS but about the shape of the
  change — a rule that *relocates* an output breaks readers that a rule which only *adds* one would
  not, and `bin/` being a superset for long enough gets it treated as one. `installer.sh` reads
  both folders and takes from `bin-console/` whatever `claim()` claims that `bin/` did not supply,
  which is derived rather than a list of the three names. **Windows was never affected**: there the
  pair is two files under two names, so rule 2 keeps `<x>.exe` in `bin/`.
- **Its own round trip is what caught that**, unpacking the built package and starting each of the
  seven commands out of the extracted payload. Worth stating as a property rather than an anecdote:
  the setup program is macOS-only, and the release workflow leaves it out until the repository
  holds Apple signing secrets, so that check is the only thing standing between a payload rule
  changing and a release shipping short.
- **`--no-video` is settled by looking in the folder, not by reading the flag.** This was got wrong
  twice, and both attempts look sufficient. A bare glob gathered a video build staged an hour earlier;
  passing acceptable suffixes still failed, because the marker goes on the *declined* build, so a
  product with no `video` feature never carries one, the empty suffix has to stay acceptable — and the
  empty suffix matches a video build too. What is actually true is whether the libraries are sitting
  there, so that is the question asked, and it decides what the generated README claims.
- **Versionless folders, versioned archives**, so `clean.sh --old` can never take one.

### Where a wallpaper pack goes

`km-display` reads a zip in the wallpaper folder as a folder of images, so a pack's natural home
looks like `assets/wallpapers/`. **One file reaching every desktop build for free is exactly what is
wrong with that:** a pack built once to look at travels into the Windows folder, the macOS bundle, the
tarball, `dist/bin`, the installer and the APK, and nothing but the APK's size note says so.

The default is `local/assets/wallpapers` — the checkout overlay, which km-app prefers over
`assets/` when run from a checkout, so `cargo run` still picks a pack up. `--zip-dest
./assets/wallpapers` is how a pack goes into a release, deliberately and by typing it, and
`check-assets.sh` then names it on every staging run.

## What a staging run says out loud

**Two mechanisms, and the split is the design.**

- **Cargo takes its own `--quiet`**, which drops `Compiling`/`Finished` and leaves every rustc
  diagnostic where it was — verified rather than assumed, with a deliberate `E0308` coming through a
  default-verbosity run in full, span and all.
- **Everything else goes through `dist_run`**, which captures to a temp file and replays the *whole*
  log to stderr on failure. `docker build`, `cargo deb` and `fetch-assets.sh` are its callers; none of
  them separates report from noise the way cargo does. The whole log rather than a tail, because these
  commands are minutes long and making somebody rerun one to find out why it failed is the cost this
  exists to avoid.

**`docker run` is deliberately not wrapped.** What the container prints *is* the report — the
`dpkg-deb --field` dump, the Depends assertions, the byte totals. Only cargo's stream inside it is a
log, so the quieting happens in there where the two can be told apart.

**`check-assets.sh` is the one thing that speaks up unasked**, and it earns it by saying what is about
to be copied. It refuses exactly one thing — two `.sf2` in `assets/soundfont/`, because the `.deb`'s
glob would ship both — and reports the rest. Inside `dist_stage_assets` its call is redirected to
stderr, because that function runs in a command substitution and its stdout *is* the file count.

Each build step prints `built in 2m14s` afterwards, from bash's own `SECONDS`. Silence is answered by
elapsed time, not by a progress bar.

### `task run` does not wait, and a bare `&` will not do it

**Task's embedded shell is `mvdan/sh`, which waits for its background jobs**, so the launcher has to
be a process that forks and exits by itself:

| platform | launcher | why |
|---|---|---|
| Windows | `cmd /c start "" "$exe"` | `start` is a `cmd` builtin. The empty `""` is the window title, which `start` would otherwise take from the quoted path, leaving no program to run |
| Linux, macOS | `/bin/sh -c '"$@" … &' sh "$exe"` | a real `/bin/sh` does not wait for background jobs |
| macOS `.app` | `open -a "$PWD/$app"` | returns at once, and gives activation and a Dock icon |

`CONSOLE=1` blocks on Windows and that is the rule rather than an exception: asking for the twin that
prints means wanting to read what it prints.

### The Taskfile is an index, never a definition

**No task may carry a flag, a default, a feature list or an ordering the command it wraps does not.**
`task test` *is* `cargo km-test` and would be a bug if it were anything else. The concrete test is the
video feature list, spelled in `tools/setup/features.sh` and `.cargo/config.toml` with `check.sh`
asserting the two agree: `task build` reaches it through a `km-build` alias rather than
spelling it, so the Taskfile is not a third copy of the one string that has already been wrong in two
places at once. **`task` staying optional is what keeps that honest.**

**A wrapped script's positional argument becomes a variable named after the script's own name for
it**, which is why the two deploy tasks take `HOST=user@box`: `deploy.sh` and `appliance-boot.sh`
both hold it in `HOST` and both spell their usage `[user@]host`. It is the one variable in the file
with no default and none available, because the only value that would serve is the address of
somebody's own house and no tracked file may carry one — so a missing host is a failure rather than a
fallback, and `_host` is what states it in the spelling that was typed. The scripts refuse it too;
what they print is their own usage line, which names a command nobody ran.

Two consequences: **`task lint` does not forward `{{.CLI_ARGS}}`**, alone among the wrappers, because
`km-lint` ends in `-- -D warnings` and an appended argument would land after that `--` and reach
rustc. And **`dist:deb`, `dist:deb:tools` and `dist:tarball` are not gated on Linux** — all three
build entirely in Docker, and without them the `verify:*` tasks were unreachable from the box this is
mostly developed on.

**One thing the Taskfile does carry that the commands it wraps do not**, and it is a stated exception
rather than a lapse: a global `env:` setting `CMAKE_POLICY_VERSION_MINIMUM=3.5`. CMake 4 refuses to
configure SDL3_ttf's vendored FreeType, which still declares a 3.0 minimum, and the failure is a
build-script panic naming neither CMake nor FreeType. **The boundary is what makes it safe**: an
environment variable that lets a third-party build system configure at all is not a flag, a default,
a feature list or an ordering — it changes nothing about *what* is produced, so `task build` and
`cargo km-build` still yield the same artifact from the same inputs. **What it costs is stated
plainly**: on a box with CMake 4 the two stop being interchangeable at the point of *failure*, with
`task build` succeeding where a bare `cargo km-build` panics — so the export stays documented in
`CLAUDE.md` for the hand-typed case, which is the only one left that this does not cover.
`tools/platform/macos/app-bundle.sh` sets the same variable for the release build it drives.

## The Windows folder

```
karaokemachine.exe   karaokemachine-console.exe   README.txt
avcodec-61.dll  avformat-61.dll  avutil-59.dll  swresample-5.dll   ffmpeg-LICENSE.txt
assets/soundfont/…  assets/wallpapers/…
```

**Why a folder and not just the exe.** `Paths::discover_asset_dir` takes the directory beside the
executable if it has an `assets` child, and otherwise the working directory — `cargo run` from the
repository root is the *fallback* case. A bare copied exe silently loses its bank, its wallpapers and
its font, and comes up on a sine test tone over a gradient with nothing saying why.

The script's first step is `fetch-assets.sh`. That is the point of it: the release step that puts the
SoundFont in the build rather than trusting whoever ran `cargo build` had fetched it.

**Deliberately not in the folder:** no SDL DLL (`build-from-source-static` links SDL3 and SDL3_ttf in,
`rusqlite` is `bundled`); no bundled font, since Windows has one at a known path and shipping one
means shipping its license; no `remote-dev` folder, the page being compiled in; no `km-pack.exe`,
since packages are built on a developer machine and installed over the API.

**The one thing it does not carry is the MSVC runtime.** `VCRUNTIME140.dll` is imported dynamically,
so the machine needs the VC++ redistributable — present on virtually every install and named in
`README.txt`. `-C target-feature=+crt-static` would remove even that, but it changes the CRT for the
vendored C in SDL3, FreeType and SQLite too.

**Two executables, and the trade that produced them was originally judged wrong.** The old argument
was that a console window is the price of keeping `--version`, `--show-paths` and `--set-password`.
That weighed a console against no command line when the answer is *both* — a library with two
three-line binaries, since `#![windows_subsystem]` is a property of a binary crate root. It also
mispriced the console: this is a fullscreen appliance under a television, so the console was a black
window sitting beside the lyrics all evening. The `README.txt` line telling people to expect it was
the tell — a feature does not need a warning.

The twin is a debugging tool rather than "the one to type", and only the words changed. Standard
handles are inherited whatever the subsystem, so a GUI-subsystem `--show-paths` prints to an inherited
console or pipe perfectly well; what the twin buys at an interactive prompt is a shell that *waits*.
The README sentence is conditional (`If there is a …-console.exe beside it`) because that one heredoc
is copied into three carriers with different contents.

**It verifies rather than asserts.** The claim a video folder makes is that it is self-contained, so
the script ends by running the staged exe with `PATH` cut to `System32`. That is a real check: a video
build with its DLLs missing does not degrade, it dies at load time with `STATUS_DLL_NOT_FOUND` and a
message naming a file rather than anything about video.

One trap: the script reads the version by running the freshly built exe, and at that moment the DLLs
have not been staged. So it puts `$FFMPEG_DIR/bin` on `PATH` for the duration.

The exe carries its own icon as a Windows resource. That build script is a **warning, not a failure**
when it cannot run: compiling a resource needs `rc.exe`, and a machine can have the MSVC linker
without it. Refusing to build the application over its icon would be the wrong trade. `winresource`
defaults `FileDescription` *and* `ProductName` to the crate name, which is only visible by looking at
a built binary, so both are set: `ProductName` is *KaraokeMachine* in all four scripts, and
`FileDescription` is each program's own name. Windows Firewall's allow-this-app dialog is where that
field is read, so it holds a name rather than the crate's `cargo` sentence — see
[`What the tool calls itself`](../decisions/foundations.md#what-the-tool-calls-itself) for every
place a name is spelled out.

### Video in a release build

**On by default in every staging script**, with `--no-video` asking for the smaller build. The cargo
feature did not move, and the distinction is the point: cargo's default answers *what must somebody
install to compile this at all*, and a staging script's answers *what does a person receiving this
folder get to play*. Getting the same answer to both was the mistake.

- **A missing ffmpeg stops the script before it builds anything**, naming both remedies. Falling back
  to a non-video build with a warning would have reintroduced the exact failure being fixed, quieter.
  The exception is a run that selected only tools with no such feature — a default that fails on work
  it could not have applied to is a demand, not a default.
- **The marker goes on the declined build** (`-no-video`, `no-video/` for the `.deb`), because two
  builds of one version must not overwrite each other and `--zip` must not clobber a zip already sent
  to somebody. `km-lyrics` and `km-wallpaper-pack` never carry it: naming a choice they were never
  offered would be a lie in a folder name.

**Four DLLs, not seven, and the list is derived rather than guessed.** `ffmpeg-next` is taken with
`codec`, `format` and `software-resampling`, so avdevice, avfilter and swscale are never linked. The
four that are are closed under dependency, read out of the import tables. Copying `bin/*.dll` would
add 29 MiB the program does not use and would stop being a statement about what it links.

**What each platform pays.** Windows: the loader searches the executable's own directory, so DLLs
beside the exe need no `PATH`, no installer and no variable. Linux: the `.deb`'s `Depends` names the
four and Debian supplies them, so nothing is staged — the one platform where video costs the shipping
story nothing. macOS: a Mach-O names each library by the absolute path it was linked at, so this is
the one that costs work.

## The Windows installer

One Inno Setup 6 installer carrying all eight products behind five component checkboxes; 75 MiB from
a 187 MiB payload.

**It gathers, it does not build.** The payload is `dist/bin/windows`, so nothing about features, DLLs,
console twins or READMEs is written down a second time. `dist/bin-console/windows` is not opened.

**One payload file is deliberately not installed: `README.txt`.** That is the exception to the
paragraph above, and it exists because gathering has a limit — the folder README is a *document about
the folder*, and an installed build is not one. It said the folder holds every executable this platform
can build, listed all eight programs, and said to remove it by deleting the folder because nothing was
registered — while being the `Read me first` Start Menu entry beside an uninstaller, a Start Menu
group, a `PATH` entry and a file association. `dist_installed_readme` writes the replacement and lives
in `common.sh` for the reason `dist_ffmpeg_license_note` does: two setup programs describing how to
remove the same product differently is what one copy prevents.

**The `.iss` names every file, and the driver reads `[Files]` back out.** Anything unaccounted for
**fails the build, by name**. `*.WebView2` is subtracted by glob rather than by one name, because
`km-remote.exe` links the same webview, and the count is printed rather than dropped in silence.

**`[Components]` is read back out the same way**, and for a reason the `[Files]` reconciliation would
not have caught: the round trip installs "everything" by *naming* the components, so a list of them
written out in the driver is a list that stops agreeing the day a product is added — and one has been:
an `assets` component arrived with `km-admin`, a verification install went on selecting the first
four, and the check failed on the executable it had never asked for. Both the summary line and the
full install come from the `.iss`, and a parse that finds no `[Components]` fails the
build rather than verifying nothing. The macOS driver had already been made to derive its component
list, its tool commands and its bundle table for exactly this reason — that same arrival left three
hand-written "six"s behind there.

**WebView2's default user data folder is beside the executable**, so a program that takes the
default puts its cache *inside* `{app}` after an install — which makes `{app}` a directory that has
to stay writable, and under `Program Files` means no webview can be created at all. The symptom is a
window that never opens rather than an error anybody could act on. Both programs name a per-user
cache directory through a `WebContext`;
`[UninstallDelete]` stays as belt-and-braces, guarding the next program that writes beside itself.

**Per-user, into `%LOCALAPPDATA%\Programs`, and no UAC prompt.** What justifies the scope is that
everything configured beyond the files is already per-user: the `.kmbuild` association goes to
`HKCU\Software\Classes` by `register.rs`'s own decision, and the PATH entry to `HKCU\Environment`. A
machine-wide install would put files where every account can see them and configure them for one.

**The `.kmbuild` association is delegated, never duplicated** — `[Run]` calls the exe's own
`--register`. A GUI-subsystem program has nowhere to print a failure, so the exit code Inno collects is
the only thing that could notice one; the entry carries no `nowait`.

**PATH is done in `[Code]`, both directions together.** `[Registry]` could add the entry, but taking
one back out of a value this installer did not create has no declarative form, so splitting the two
would invite drift. `RegWriteExpandStringValue`, because a real user's `Path` routinely contains
`%USERPROFILE%` and rewriting it as a plain string would expand those permanently. This is one of the
two places Inno being chosen over NSIS actually paid: NSIS caps strings at 1024 in a stock build, so
reading a long `Path` back truncates it.

**Verification is a round trip run at the end of every build**: install silently into a scratch
directory with `/TASKS=""` so it cannot touch this machine's PATH or associations, run `--version` on
all eight with a bare PATH and assert the **exit status** (which is what proves every imported DLL
resolved from the install folder), uninstall, assert the directory is gone. Then install `remote`
alone and assert it carries neither the assets nor the DLLs — without that half, the component wiring
could quietly stop saving the 140 MB it claims to.

Four details learned by getting them wrong:

- **A round trip that fails between the install and the uninstall leaves a registration behind.** The
  scratch tree is removed by an `EXIT` trap, and for a long time that was all it did — but a per-user
  install writes itself under `HKCU\...\CurrentVersion\Uninstall`, and `unins000.exe` is the only
  thing that takes the entry back out. Deleting the folder deletes the uninstaller with it. What
  survives is a row in Settings > Installed apps naming a directory that is gone, with no icon
  (`UninstallDisplayIcon` points into it) and an Uninstall button that cannot work — reported, the
  first time it happened, as a Windows bug. The trap runs any `unins000.exe` it finds and waits for
  it, bounded, before removing anything. It lives in `tools/platform/windows/inno.sh` with the rest
  of the harness, because a second driver inherits the lesson rather than re-earning it.
- **Inno's `Setup.exe` can hand the work to a second process**, so its exit is not the end of the
  install. The loop waits for `unins000.exe`, which is written last. The uninstaller relaunches itself
  from a temp copy, so its assertion is on the directory disappearing, polled.
- **`ISCC.exe` with no arguments prints usage and exits non-zero**, so the version probe took the whole
  script down under `set -o pipefail`. The `|| true` on that pipeline is load-bearing.
- **`MSYS2_ARG_CONV_EXCL='*'` is required for every call into a Windows program.** Git Bash rewrites
  arguments that look like POSIX paths, so `/DPayload=...` arrives as `C:/Program Files/Git/DPayload=...`.

**A fourth `[Tasks]` entry writes the instrument-bank request**, ticked by default and gated on the
`machine` component: two lines of JSON from `{#Generated}` into `%APPDATA%\karaokemachine\config`,
with `onlyifdoesntexist uninsneveruninstall` so a reinstall cannot reset an attempt count and the
uninstaller cannot break its own closing promise. The size and the license come from
`tools/setup/soundfont-banks.sh` as `/D` defines; the license's apostrophes are doubled by the driver
because it is pasted into a Pascal literal in `UpdateReadyMemo`, which is where the terms are shown.
The name and the size are deliberately *not* escaped — they also go into a `[Tasks]` description,
which is plain text, and neither can carry an apostrophe.

**The round trip asserts only the negative half**: that a `/TASKS=""` install leaves nothing in
`%APPDATA%`. The positive case is untestable there — Inno resolves `{userappdata}` through the shell
folders, which no environment variable redirects, so proving it would mean writing a real request
into the developer's own install and their next start would fetch 262 MiB. What is asserted at build
time instead is that the generated file exists and names the row the table marks `recommended`.

Unsigned, so a recipient sees SmartScreen's *"Windows protected your PC"* and clicks through.

## The macOS installer

One `.pkg` carrying all eight products behind six visible ticks; 80 MiB from a 150 MiB payload.

**A seventh component package with no payload at all.** `soundfont` is built with `pkgbuild
--nopayload`: no `--root`, no `--install-location`, and its whole content is a postinstall that
writes the same instrument-bank request into the console user's `~/Library/Application Support`.
It is a second array in the driver rather than a seventh entry in `COMPONENTS`, because everything
that array drives — `claim()`, `install_location`, and the two checks reconciling what was staged
against what was archived — asks a question about files that a payload-free component cannot answer.

Two things about that postinstall are worth knowing:

- **It runs as root and the request belongs to a person**, so it resolves the console user with
  `stat -f %Su /dev/console` and `dscl`, and chowns both the folder it may have had to create and
  the file. An install driven over SSH has root on the console and writes nothing, which is the
  honest answer when there is no home directory it could mean.
- **Nothing in it may fail the install.** `require-scripts="true"` is set, so every step falls out
  to `exit 0` — the same tolerance the builder's `--register` nudge has. "We could not write down
  which bank you wanted" must not become "the karaoke machine would not install". All four
  postinstalls are now `sh -n`-checked before they are packaged, since a heredoc that will not parse
  otherwise fails on somebody else's Mac with nothing on screen naming the line.

**Five of the six carry a payload, and they are five because of where they land.** `pkgbuild` takes
one `--root` and one `--install-location`, and `/Applications` and `/usr/local/karaokemachine`
cannot share one — so the count is forced rather than chosen. One of the five, `docs`, is hidden and
always selected and holds the READMEs and the uninstaller, which means the one way to take this off
again is not something a tick could decline.

`com.rrgmc.*` and not `com.karaokemachine.*`: package identifiers and bundle identifiers are
different namespaces that must not be conflated.

**All six bare executables go in `tools`, not symlinked out of their bundles.** `lib/` decides it:
every one was staged with `@executable_path/lib`, so splitting them means either a second 15 MB copy or
a package that cannot stand alone.

### The claim table is the staging loop

The Windows driver reconciles by parsing `[Files]` back out of the `.iss`, because there the
description and the payload are two things that can disagree. Here they are one: `claim()` maps a
payload entry to a component, and **the staging loop is a walk driven by that function**, so an entry
cannot be staged without being claimed. An unclaimed entry fails the build by name — proven
non-vacuous by planting a stray file and watching it refuse.

Three assertions sit behind it: every component root is non-empty (a `case` arm matching nothing
produces a smaller package and no error); the payload's file count equals the staged count plus three
and minus the payload's own README, **with the two adjustments kept separate because letting them
cancel is how the check passes while meaning nothing**; and each archived payload equals what was
staged, read back with `pkgutil`, which asks the artifact rather than the intention.

**`.DS_Store` is this platform's `*.WebView2`.** A top-level one is subtracted by name; a *nested* one
is refused outright, because `pkgbuild` drops every `.DS_Store` at any depth by its own default filter
— measured, not taken from the man page. Left alone, one inside a bundle would be staged, dropped, and
surface a screen later as a mismatch rather than as its cause. It does **not** break the seal —
`.DS_Store` is in `codesign`'s own default exclusions and is never sealed — so the mismatch is the
whole of the reason.

### The second bundle, and the one thing that can be wrong with it

`KM Stream.app` is what `--stream` has instead of the Start Menu entry Windows gives it
and the desktop action Linux gives it, and the reason it is a bundle rather than a line in one is that
a macOS manifest has nowhere to put a launch argument. It holds a plist, an icon and a four-line
script, and it copies nothing: what it runs is the binary inside `Karaoke Machine.app` beside it.

- **`exec`, never a symlink**, which is the reason `/usr/local/bin/karaokemachine` is a shim — Rust's
  `current_exe()` on Apple is `_NSGetExecutablePath` with no realpath, so a machine that sees this
  bundle rather than the real one takes neither branch of `discover_asset_dir` and comes up on a test
  tone. `exec` makes the kernel record the real path.
- **The path is resolved from the script's own location**, so the pair works unzipped into a Downloads
  folder as well as installed under `/Applications`.
- **Its `CFBundleIdentifier` is its own.** LaunchServices keys a bundle on that, and two bundles
  sharing one are one application to the Finder, the Dock and `open`.
- **So is its icon**, and that is the one thing it holds that the bundle beside it does not: they sit
  together in `/Applications` and in the Dock, where one mark on both leaves two entries differing
  only by the name under them. `Info.stream.plist` names `karaokemachine-stream`, and
  `dist_stage_macos_bundle` is handed the matching `.icns`.
- **A row in `BUNDLE_STARTS` is what proves it reaches the machine**, and it is the only thing about
  this that can be wrong: a relative path into a bundle renamed or moved is a launcher that starts
  nothing and says nothing. `--show-paths` returns before anything is opened, so the check asks the
  question without starting an encoder.

### Two `pkgbuild` traps that are silent when wrong

- **`BundleIsRelocatable` defaults to `true`.** With it on, Installer places a `.app` wherever an
  existing bundle with the same `CFBundleIdentifier` already is — so somebody who unzipped the app into
  `~/Downloads` gets *that* copy upgraded and `/Applications` stays empty, with no error anywhere. Each
  bundle component gets a component plist, **derived with `pkgbuild --analyze` rather than committed**,
  so a bundle that changes shape cannot leave a stale one behind.
- **The check for it is not the obvious grep.** `<relocate/>` is present in *every* `PackageInfo` — it
  is the *list* of bundles that may move, and an empty one is the good case — so grepping for the word
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

The machine is the only one of the seven that reads anything relative to its own executable, so it is
the only one that gets a shim. **The round trip asserts both halves** — that the shim finds the assets
*and* that a symlink does not — because an assertion on the shim alone would go on passing the day
somebody canonicalises `current_exe()` and makes the shim pointless.

**`/usr/local/bin` is created by the postinstall and never archived.** A payload naming it puts its
mode and owner in the bill of materials, and Installer applies BOM directory entries to directories
that already exist — on an Intel Mac that directory is Homebrew's, so ours would quietly break `brew`.

### Uninstalling, which macOS does not do for you

`uninstall.sh` removes the application bundles, the folder, the `/usr/local/bin` entries and the
receipts, and **nothing it identifies by name**. A bundle is ours if its `CFBundleIdentifier` starts
`com.karaokemachine.`; a `/usr/local/bin` entry is ours if it resolves into `$PREFIX` or carries the
shim's marker line; a receipt is ours if its id starts `com.rrgmc.karaokemachine.`. A blanket
`rm` of six names out of a directory this package did not create is exactly what an uninstaller must
not do — and a list of names is also the thing that cannot survive a rename, which is the decision
`An uninstaller finds its own work, and never by name`.

**A test of that ownership check found a real bug:** `readlink` returns the target as written, so a
*relative* symlink compared against an absolute prefix reported every entry as somebody else's. The
postinstall happens to write absolute targets, so nothing would have shown it.

**The candidate set was the half that stayed hardcoded, and it cost three orphans.** The ownership
test was right from the start; what was wrong is that it was only ever asked about names a fixed
list already knew, so a bundle or a symlink under a superseded name was never offered to it. Two
installs either side of a bundle rename left `KaraokeMachine Assets.app`,
`KaraokeMachine Package Builder.app` and `KaraokeMachine Remote.app` in `/Applications`, dangling
`km-assets` and `wallpaper-pack` links in `/usr/local/bin`, and an `assets` receipt beside the live
`admin` one. Scanning `/Applications`, `$BINDIR` and `pkgutil --pkgs` and asking the existing test
about every entry finds all six without being told any of them exists.

It was right from the first build and **unfindable** — named only in two Installer panes, not even in
the README beside it. `Uninstall KaraokeMachine.command` is the tarball's answer ported: `.command` is
the extension the Finder hands to Terminal, and it is a wrapper, so `uninstall.sh` stays the only thing
that removes anything. Four things in it are load-bearing: `cd "$(dirname "$0")"` first (Terminal
starts in the home directory); the dry run **before** the password prompt; a tty check with the
confirmation defaulting to no; and `exec sudo`, so the prompt lands in the window showing the list.

The round trip runs it with stdin closed and asserts it removes nothing **and names the direct
command**. That second assertion is load-bearing and not obviously so: without the tty check the `read`
gets EOF, the answer is empty, and the defaults-to-no branch prints *"Nothing was removed"* anyway — so
the first assertion passes on a broken wrapper.

**The "your songs are left alone" message is written once and said twice**, from
`pkg/data-locations.txt`, because macOS has no uninstaller UI so the sentence must appear in both the
installer's closing pane and the uninstaller's output. Running the generated script is what caught the
substitution bug, and its shape is worth knowing: the marker was also mentioned in the template's own
header comment, so a plain replacement dropped ten lines of prose into a `#` comment. The result is
still *valid shell*, so `sh -n` passes it. The build now asserts each marker occurs exactly once.

**The panes are sniffed, not declared.** `mime-type="text/html"` does not settle it — Installer sniffs
the file's *data*, and a file beginning with an HTML comment fails that sniff and renders as plain
text. Both panes begin with `<!DOCTYPE html>` before anything else, comments included, and the driver
asserts it on every build.

### Signing

Unset, signing is ad-hoc. `KM_SIGN_IDENTITY` set, `dist_codesign` signs with it — `--options
runtime`, `--timestamp` — and every site that signs calls it. **One function rather than four spelled
`codesign` lines**, because a bundle whose dylibs are ad-hoc and whose executable is Developer ID is
one notarization refuses while naming neither file.

`KM_SIGN_INSTALLER_IDENTITY` is separate because it is a separate certificate type. The half-signed
combination is refused up front: Gatekeeper judges the archive, so signed bundles inside an unsigned
`.pkg` buy a recipient nothing.

**`installer.sh --notarize` is the one path where all three have committed defaults**, so `task
dist:setup:notarized` needs nothing set. They are applied inside the `NOTARIZE` gate rather than at
the top of the file, because `dist_signing` keys off `KM_SIGN_IDENTITY` being non-empty and a default
outside that gate would make every build sign. `KM_SIGN_IDENTITY` is the one of the three that is
**exported**, because `tools/dist/bin.sh` runs as a child process and `dist_codesign`
signs there, so an unexported assignment would stage an ad-hoc payload under a Developer ID wrapper —
the half-signed shape above, found only by the round trip after a full staging run.

Four things learned by running it:

- **`grep -q` under `set -o pipefail` reports the opposite of what it found.** It exits at the first
  match, SIGPIPEs whatever is upstream, and `pipefail` makes the status 141. The same flaw was in
  `dist_verify_macho_portable`'s absolute-path check, where it fails in the *dangerous* direction: a
  real absolute load path makes `grep` match, exit, and the `if` reads 141 as *no match*. Both capture
  output first and match afterwards.
- **`-R` needs its value quoted.** `subject.OU = TEAMID` is a syntax error; `= "TEAMID"` is not.
- **`--options runtime` must be applied at every signing call, not to the bundle alone.** Sealing does
  not add the hardened runtime to code already signed inside it, and `--verify --deep --strict` does not
  check that it is there — so a partial application passes every check here and fails notarization.
- **A locked keychain makes signing block on an unlock prompt with no output**, because `dist_run`
  buffers and replays only on failure. `productbuild` runs outside `dist_run` when signing.

**Notarizing rather than only signing is what removes the dialog**, in the platform's own words:
signed-but-not-notarized is `rejected / source=Unnotarized Developer ID`; the same archive notarized
and stapled is `accepted`, and `stapler validate` passes on a quarantined copy, which is what says the
ticket travels in the file rather than being fetched.

**Verification needs no password.** A system-domain `.pkg` installs to `/` and needs root, so the
default round trip expands the archive, inspects what would land, then builds the installed layout in a
temporary directory and runs it. `pkgutil --expand-full` is **undocumented** — in neither the man page
nor `--help` — so it is probed rather than trusted, with `--expand` plus `cpio` behind it. `--install`
is the opt-in real thing.

## The remote's own installers

A second carrier on each desktop platform, holding `km-remote` and nothing else; 5 MiB from a 20 MiB
payload on Windows. The product decision is `A setup program for the remote alone`; this is how it is
built.

**The payload is `dist/km-remote/<platform>/km-remote-<version>-<triple>`**, which `tools/dist/cmd.sh`
already stages — five files, two seconds — rather than `dist/bin/<platform>`, which is every product
and a six-minute run. The folder is *found* rather than named: `dist_staged_dir` takes a version and
the version comes out of the binary inside the folder, so the driver globs for one match and refuses
two, the shape `release.sh`'s `one_match` uses. A `-no-desktop` payload is refused by looking for
what `cmd.sh` stages only alongside a window — the console twin on Windows, the `.app` on macOS: a
setup program installing a remote that cannot open one is not what it promises, and that is the same
refusal the all-in-one makes about a payload with no ffmpeg in it.

**What the two drivers share is in `tools/platform/windows/inno.sh`**: the compiler search and its
Inno-6 assertion, the two `sed` programs that read `[Files]` and `[Components]` back out of a script,
the payload reconciliation, and the install-run-uninstall harness. The harness is the piece that had
to move — see the orphaned-registration bullet above. `tools/platform/windows/webview2.iss` is the
same bargain for the `[Code]` section: the three-registry-view runtime probe and the bootstrapper
download, `#include`d by both scripts, with the two sentences that differ taken as defines. Each
script keeps its own `NeedsWebView2`, because Pascal Script resolves in order and the question
genuinely differs — one asks which components were ticked, the other asks nothing.

**Two `.iss` files rather than one compiled twice.** The drivers read the script with `sed`, which
knows nothing about the preprocessor, so a file whose `[Files]` and `[Components]` depended on a
define would hand each driver the other's lines — and that reconciliation is the only thing standing
between a carrier and a silently dropped file. The two also disagree in nine sections, which is not a
variant.

**The coverage check is the same rule with a different exclusion.** The all-in-one subtracts the
folder `README.txt` and installs `README-*.txt`; the remote-only carrier subtracts
`km-remote-console.exe` and installs the folder README *as* `README-km-remote.txt`, because that
document is the remote's own and is right wherever it is read. The short one beside it is
`dist_installed_readme windows km-remote` — a product argument rather than a third platform, so four
setup programs cannot come to describe removing the same two products four ways.

**Two assertions with no sibling in the all-in-one.** The ffmpeg list is asserted *absent* from the
script rather than present in the payload: the remote links none of it, and one careless `[Files]`
line would turn a 5 MiB download into a 40 MiB one with nobody deciding to. And `AppId` and `AppName`
are compared against `installer.iss` and the build refuses when either matches — Inno decides
upgrade-versus-second-copy by `AppId`, so the copy-paste that shares one is the only realistic way a
remote install comes to remove the karaoke machine. The install folder and the Start Menu group both
follow `AppName` in each script, which is why two comparisons cover four values.

**`dist/setup/windows/km-remote-generated` and not `generated`.** The all-in-one clears its own
generated directory on every run, so one shared directory would mean whichever installer built last
owned the other's README.

**The round trip is the all-in-one's, shorter and mostly negative**: install, run `--version` on a
bare `PATH`, then assert the absences that define the carrier — no console twin, no other product, no
`assets/`, none of the four DLLs, no ffmpeg licence beside no ffmpeg — then both READMEs, then the
planted `*.WebView2` folder, then uninstall and assert the directory is gone.

### The macOS half

`tools/platform/macos/pkg.sh` is what `inno.sh` is one platform over, and the selection is driven by
the same question — which of these, copied and drifted, produces something that *looks* fine?
`pkg_resolve_signing` (two certificates, three states, every refusal, and the `export` that makes the
staging subprocess sign the same way), `pkg_component_plist`, the template fills with their
occurs-exactly-once guard, the two pane renderers and the doctype assertion, `pkg_expand`'s probe of
the undocumented `--expand-full`, and `pkg_notarize`.

**Two component packages, because `pkgbuild` takes one `--root` and one `--install-location`**:
`com.rrgmc.km-remote.app` into `/Applications`, `com.rrgmc.km-remote.docs` into
`/usr/local/km-remote`. The count is forced rather than chosen, and the second is hidden and always
on for the reason the all-in-one's `docs` is.

**The receipt namespace is the coexistence mechanism, and it is `com.rrgmc.km-remote`.** The
all-in-one's uninstaller forgets receipts by the `com.rrgmc.karaokemachine.` prefix, so a
component here that borrowed that namespace would have its receipt taken by a package that never
wrote it. The round trip greps the expanded archive for that prefix and fails on a hit.

**The bundle identifier is the opposite answer to the same question.** `com.karaokemachine.remote`
stays, because it names the application and two bundles may not share one — so both packages place
the same `/Applications/KM Remote.app`. Each uninstaller therefore asks whether the other's receipt
is present before taking it, and says so on the line where it does not.

**The payload is two paths, because that is the shape `cmd.sh` stages.** `KM Remote.app` sits beside
the versioned folder rather than inside it — a product's platform folder holds exactly one bundle,
its own, and `dist_stage_macos_bundle` takes a second one as the fossil of a rename. So the
application comes from one path and the licence texts from the other, and the absent bundle is what a
`-no-desktop` staging looks like here.

**The claim table is a `case` with one arm and two named exclusions**: the two licence texts are all
the folder is asked for, and the bare `km-remote` and `README.txt` — the terminal form and the folder
document — are the exclusions. The file-count reconciliation subtracts the skipped entries on one
side and adds the bundle's own files and the three generated ones on the other, each kept separate so
they cannot cancel.

**Each half can be verified only on its own platform.** `pkgbuild`, `productbuild`, `pkgutil`,
`codesign`, `spctl` and notarization are all macOS, and its `--install` arm needs a password on the
Mac it runs on; the Windows half installs and uninstalls on the developer's own account there. That
round trip is the only thing standing between a payload rule changing and a package shipping short.

**Coexistence is the one thing neither round trip covers**, because it needs both packages on one
Mac at once. Each uninstaller is run first in turn: the one that runs first keeps
`/Applications/KM Remote.app` and says so, because the other carrier's receipt still claims it, and
the one that runs second takes it.

## The four macOS bundles

`Karaoke Machine.app`, `KM Package Builder.app`, `KM Remote.app` and `KM Admin.app`.

**Why a bundle rather than the bare binary.** A bare Mach-O is a terminal program: no icon in the Dock
or Finder, not double-clickable, no `Info.plist` — so nothing names the window and there is no
`CFBundleIconFile` to draw.

**One helper, four callers.** `dist_stage_macos_bundle` and `dist_seal_macos_bundle` in `common.sh`.
The contract worth remembering is the icon's: **the `.icns` is copied under its own basename, and the
plist's `CFBundleIconFile` must equal that name without the extension.** A mismatch is an error
nowhere — it draws a generic icon and says nothing.

**Every bundle is sealed, not only a video one.** It costs milliseconds and buys
`dist_verify_macho_portable` running on that build too, which is what would catch a stray absolute load
path in a bundle that carries no ffmpeg but still links SDL.

**Staged from the pristine build output, never from the folder's copy.** That copy has already had its
load commands rewritten to `@executable_path/lib`, so handing it over a second time would send it
hunting for `@rpath/...` dependencies and finding none. Two copies of one build, two rpaths, one
`cargo build`.

The plists differ, and each difference is a decision:

- **The remote declares no document type.** It opens no document and takes no positional argument. The
  builder's plist argues that declaring a type you cannot open is worse than declaring none; this is
  that argument in the negative. Hence `--no-desktop` takes the bundle **and** the window together — a
  document double-clicked arrives as an Apple Event, so a bundle around a build with no event loop
  would be handed a corpus and have nowhere to put it.
- **The builder declares four `NS*FolderUsageDescription` keys** because it walks a folder of somebody's
  songs; the remote declares none, reading nothing the user chose.
- **Three of the four declare `NSLocalNetworkUsageDescription`** — the builder, the remote and the
  assets tool. Browsing mDNS and then talking to a LAN address is a prompt since macOS 14, raised
  against the *bundle*: the same executable run from a terminal inherits the terminal's grant and is
  never asked. **The machine is the one without it, and that is correct** — it advertises and never
  browses, so nothing in it constructs a `Watcher`.
- **`LSApplicationCategoryType` is the one key all four declare**, and the only difference is which:
  Music for the machine and the remote, Utilities for the builder and the assets tool. It is advisory
  — nothing at run time reads it — so a manifest that omits it, or names a category macOS would
  silently ignore, is caught by `installer.sh` rather than by a build log: it reads every
  `Info*.plist` by glob in the same pass that checks `LSMinimumSystemVersion` against the
  Distribution. The reasoning is in `What a macOS bundle says it is for`.

**A bundle has a TCC identity of its own and a folder does not**, which is only findable on a Mac: the
builder's first launch stops on a Desktop-access prompt *before* it serves a page, so the window is
white until somebody answers. The keys do not remove the prompt and are not meant to — they make it say
why. **A newly built unsigned bundle is also slow the first time it is opened** while Gatekeeper
assesses it, long enough that a first launch looks like a hang; the socket is bound throughout, so the
symptom is a connection accepted and never answered.

### Making the macOS video builds portable

**The closure is walked, not listed**, and that mattered more than expected. `km-video` links four
ffmpeg libraries. Against Homebrew's ffmpeg those four pull in nine more — x264, x265, SVT-AV1, libvpx,
dav1d, LAME, Opus and OpenSSL's two — for **13 dylibs and 33 MB**; against the pinned LGPL build they
pull in nothing at all, for **4 dylibs and 15 MB**. The same code produces both, because it starts at
the binary and follows every non-system load command. That is what made the license question answerable
by *changing which ffmpeg is built* rather than by editing anything here.

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
  staged afterwards invalidates it — which is how the previous point was discovered.

**The license question this raised.** Nothing was redistributed before; now something is. Homebrew's
ffmpeg is `--enable-gpl` with x264 and x265, so the first version shipped a GPL bundle while Windows
shipped LGPL — and **the application cannot call either encoder**, every codec context in `km-video`
being a `Decoder`, so it was 25 MB of dead GPL weight. `fetch-ffmpeg.sh` builds a pinned LGPL ffmpeg
from source on macOS instead. Source rather than a download because **nothing publishes what this
needs**: BtbN has no macOS target, evermeet ships a static GPL command, ffmpeg.org is source only.

## The Debian package

`.deb` for Debian 13 amd64, built in a container. **`FROM debian:13-slim` is the compatibility floor**
and the only thing to change to retarget. Why a container and not the WSL Ubuntu on the same box: that
one is newer than Debian stable, so a package built there installs on trixie and then refuses to start
— "the .deb does not run on Debian" is the one way this deliverable can be wrong while looking finished.

**The layout is `/opt`, and that is a decision.** Split it the conventional FHS way — binary in
`/usr/bin`, assets in `/usr/share/karaokemachine` — and *neither* branch of `discover_asset_dir` finds
them: the machine comes up on its sine test tone, silently. So the package keeps them together, with a
`/usr/bin` symlink from the postinst. The symlink costs nothing because `current_exe` reads
`/proc/self/exe`, which resolves *through* it — verified with a C probe before the layout was chosen.

The desktop entry and the `hicolor` icons are what make it an installed application rather than a
binary in `/opt`. `Icon=karaokemachine` is an icon-theme *lookup*, not a path, which is why each size
goes to its own directory. No cache-refresh call in `postinst` — the two relevant packages ship dpkg
triggers on exactly those directories.

**`depends` is written out, and `$auto` is the thing it replaces.** `dpkg-shlibdeps` derives only
`libasound2t64, libc6` from this binary — SDL3 is linked statically *and* opens X11, Wayland and GL
with `dlopen`, so none of them appears in the dynamic section for it to see. It would also resolve
the ffmpeg libraries the package *carries* against the build image's `-dev` packages, putting back
the dependency the bundling removes, so the list is written out and `$auto` is gone from the default
build. `deb.sh --system-ffmpeg` is the variant that still asks for it, linking the distribution's
ffmpeg and taking a derived `libavcodec61` with it.

**What replaces it is a `DT_NEEDED` allowlist over the shipped binary**, in `deb-in-container.sh`:
an X, Wayland, VA-API, VDPAU or OpenCL library there fails the build. That is the requirement in
[`An appliance install puts no X library on the box`](../decisions/distribution.md#an-appliance-install-puts-no-x-library-on-the-box)
stated as a check, and it is what notices a new link-time dependency now that nothing derives one.

**Where ffmpeg shows itself moved with it.** A package carrying its own libraries Depends on none, so
`Depends` says nothing about whether video reached the binary; `deb-in-container.sh` reads the
package's *contents* instead, asserting `opt/karaokemachine/lib/libavcodec.so.*` and
`libopenh264.so.*` for a bundled build and the `Depends` field for a system one — each in both
directions, so a run that took the wrong variant cannot pass the wrong test.

**Every one of those matches captures its producer's output and searches it afterwards**, for the
reason `tools/dist/common.sh` gives: under `pipefail` a `grep -q` that matches kills `dpkg-deb` with
SIGPIPE, and the pipeline's 141 makes the `if` read a match as no match. Over a seventy-megabyte
package the producer loses that race every time, and the direction is the danger — each check would
wave through exactly what it exists to stop.

**What container verification proves, and what it cannot.** It proves the layout, the symlink, that apt
can satisfy the derived `Depends`, and that asset discovery resolves through the symlink to the bundled
bank. It **cannot** prove the bank is *opened*: a container has no sound device, and the run says so
honestly rather than pretending.

### A second package, from a manifest that is not the machine's

`deb.sh --tools` builds `karaokemachine-tools` — `km-package-builder`, `km-remote` and `km-admin` in
one package the machine Recommends, per
[`The three tools are a package of their own`](../decisions/distribution.md#the-three-tools-are-a-package-of-their-own-which-the-machine-recommends).
It shares `deb-in-container.sh` with the machine because it shares everything around the build: the
image, the target volume, the checkout guard and the report. What differs is which manifest cargo-deb
is pointed at, `[package.metadata.deb]` living on `km-package-builder` and naming the package for
what it holds.

**km-admin comes from the second cargo workspace**, so the tools build is two `cargo build` calls
under one `CARGO_TARGET_DIR` — which is what lets one asset list name all three under
`target/release`.

**It is a release carrier, so `task dist:deb:tools` is a task of its own rather than
`dist:deb -- --tools`.** `dist:app:linux` names it beside the other two, and `tools/dist/release.sh`
prints it to somebody about to type it. The package goes to `dist/karaokemachine-tools/linux/`,
a folder rather than a filename marker, both packages carrying a name of their own.

**`$ORIGIN/../lib` is the whole of why they live in `/opt`.** The builder links the four ffmpeg
libraries and carries none; the machine's package has them at `/opt/karaokemachine/lib`, and an
rpath from `/opt/karaokemachine/tools/` reaches them. `/usr/bin` could not, so the names there are
symlinks written by `postinst` — and `postrm` reads each one's target before removing it, so an
upgrade cannot take away a link of the same name that somebody else owns.

**What the verifier proves is that the rpath resolves**, which is the one thing that cannot be read
off the package: `verify-deb.sh --tools` installs both `.deb`s in a clean container and canonicalises
what `ldd` reports for `libavcodec`. `ldd` prints the rpath as written — `tools/../lib/…` — so
comparing the text rather than the file is how a check comes to fail on a package that is correct.

### The warm cache lies when there is more than one checkout

One volume, `karaokemachine-deb-build`, holds `/build/target` for `deb.sh`, `tarball.sh` and
`check.sh` in every checkout and every worktree. One rather than one apiece is what makes a new
worktree cheap: SDL3, SDL3_ttf and the bundled SQLite are minutes of C, and a shared volume has them
compiled already.

The price is that cargo cannot tell two checkouts apart. Each bind-mounts its own root at `/src` and
fingerprints record that path, so a warm cache will hand a run artifacts compiled from different
source at the same location. In a check that is a baffling compiler error, and the answer is to
clean. In a release artifact it is a package built quietly from another checkout's code, and
`deploy.sh` installs it on the appliance. So the release paths clean when the volume's stamp names
somebody else — the workspace's own crates only, leaving the third-party half that costs the minutes.

**Incremental compilation is off in the image**, because the volume outlives the containers and they
build once each. Incremental pays for itself on the second build of an edited crate; there is no
second build here, and its per-crate caches accumulate under the debug profile across checkouts and
toolchains — disk written on every run and read on none. The release profile the carriers are built
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
- **Size, measured.** `ldd` over Debian's `libavcodec.so.61`: **93 shared libraries, 97 MiB** — x264,
  x265, rav1e, libjxl, and behind them harfbuzz, fontconfig, pango, cairo and glib, because those
  codecs want text and images.

So ffmpeg **7.1.5** is built, exactly what trixie ships, so both Linux carriers decode through the same
code. The configure line matters less than the one flag that does the work:

**`--disable-autodetect` is the rule: every decoder ffmpeg implements itself, none that needs a
third-party library.** Deliberately not a hand-picked codec list — a list has to be revisited every
time somebody auditions a file it did not anticipate, whereas this is a property with a reason.

**The rule is about decoders, and there is one encoder outside it.** A streaming machine has to
produce H.264 and ffmpeg implements no encoder for it, so `--enable-libopenh264` asks for the one an
LGPL configure line may have. What that costs the tarball is a fifth library in `lib/`: openh264 is
copied into the prefix beside the four it is linked by, under BSD-2 with its text in `LICENSES/`, so
the folder still asks the user's machine for nothing. What comes out links `libc`, `libm`, `libz` and
`libstdc++`, the last of those because openh264 is C++ where everything else here is C. libstdc++ is
not carried: a bundled one older than the system's breaks any C++ loaded after it, and it is on every
distribution the tarball claims.

**The pin lives in `tools/setup/ffmpeg-pin.sh`**, sourced by both the macOS and Linux builds. They
reached the same version, checksum and load-bearing flag independently, days apart — good evidence the
conclusion is right and poor practice to leave as two files: they had already drifted by
`--enable-zlib` before anybody compared them, which meant a `.mov` with a deflated header played out of
the macOS bundle and not out of the Linux tarball. **What is shared is the definition, not the
procedure.**

Two details worth not rediscovering:

- **`nasm` is not optional.** Without it configure quietly builds a C-only decoder. That is not a
  failure anybody sees at build time — it is a 1080p file dropping frames on the appliance, found much
  later and blamed on something else.
- **The binary is compiled against this prefix**, not Debian's headers. Building against one 7.1.5 and
  shipping another would *probably* work; the failure mode of "probably" is an undefined symbol at load
  time on somebody else's machine.

### RPATH, twice

`patchelf --set-rpath '$ORIGIN/lib'` on the binary, and `--set-rpath '$ORIGIN'` on each bundled
library. **The second is easy to miss: RUNPATH — what a modern linker emits — is not inherited by an
object's own dependencies.** The binary finding `lib/` says nothing about `libavformat` finding
`libavcodec` beside it, and the symptom is a tarball that works on the build machine and fails
everywhere else.

`patchelf` on the staged file rather than `-C link-arg=-Wl,-rpath` in `RUSTFLAGS` for a cache reason:
`RUSTFLAGS` is part of cargo's fingerprint, so setting it would make the tarball and `.deb` builds
invalidate each other in the shared volume.

Libraries are copied **under their sonames as real files**, not a versioned file plus a symlink chain:
`DT_NEEDED` records the soname and nothing looks for the longer name.

### What is asserted before it ships

- It starts the staged binary under `env -i` — stronger than a stripped `PATH`, because it also drops
  `LD_LIBRARY_PATH`, exactly the variable that would make a broken RPATH look fine on the build
  machine.
- A video build must resolve `libavcodec` **out of `./lib`**; a `--no-video` build must have staged no
  `lib/` at all.
- No bundled library may link `libx264`, `libx265`, `librav1e` or `libjxl` — the license claim checked
  rather than trusted. The plausible way it breaks is somebody adding a `-dev` package to the image,
  after which `--disable-autodetect` is the only thing standing between this and a GPL dependency.
- The bundled `libavcodec` must carry `libopenh264`, asserted the same way. A configure that dropped
  the flag leaves a folder that decodes and plays perfectly and cannot stream, which is discovered by
  whoever first runs `--stream` out of it.

### The verifiers, and why only one may prewarm

`verify-tarball.sh` installs only the runtime libraries `README.txt` tells a user to install, so a pass
means that prose list is correct *and* complete — the part of a tarball no `dpkg` enforces. `--image`
is what makes the Fedora and Arch lines more than a guess.

Images are prewarmed and tagged by a hash of the *generated* Dockerfile text. Hashing the generated
text rather than the inputs separately is the point: base ref, package list and install command are all
covered by construction, so there is nothing to forget to add to the hash. **No package name is written
anywhere in `verify-image.sh`** — deliberately, because a prewarmed image is exactly where somebody
later adds "just one more package" to turn a red verification green.

**`verify-deb.sh` cannot be treated the same way**, and this is the sharp part. It asserts
two things: that apt can satisfy the `Depends` from the archive, and that the `Depends` list is
**complete** — which holds only because the image starts with nothing, so an omitted library makes
`--version` fail. Property two does not survive prewarming the closure, and an omitted `Depends` is
precisely what `$auto` gets wrong, since it cannot see what SDL `dlopen`s. **So the closure is never
prewarmed: not as a mode, not behind a flag, not "just for iteration".** A step called `verify` that
cannot detect a missing dependency is worse than no step, because it consumes the attention a real
check would have received.

What it caches instead installs nothing: the apt index, and the downloaded `.deb` files in a named
volume. One trap — official Debian images ship `docker-clean`, whose `DPkg::Post-Invoke` deletes the
archives after every install, so the cache volume would be emptied on the way out.

**Prewarming costs freshness, and that is made visible rather than hidden:** the verifier prints the
image's age on every run, `--refresh` rebuilds it, and `--no-prewarm` goes back to installing at run
time. `--no-prewarm` earns its keep for a second reason — a branch nothing ever takes is a branch that
has quietly stopped working.

`prewarm.sh` builds all of it up front so a cold machine pays once and deliberately. **It must never
become a required step**, and it is not: every consumer keeps its own on-demand guard and calls the
same helper. It warms images only — the cargo cache is left cold on purpose, because SDL3 and the
bundled SQLite compiling into `/build/target` would turn this into a twenty-minute command nobody runs.

### What this carrier deliberately does not do

No systemd unit, no `karaoke` user, no DRM master, no `/usr/bin` symlink. The appliance is the
package's job. `install.sh` does the one thing a folder can honestly do — a desktop entry for one user
with `Exec` rewritten to wherever they unpacked it, and `--uninstall` to undo it. A tarball that wrote
into `/usr` would be a package manager keeping no records.
