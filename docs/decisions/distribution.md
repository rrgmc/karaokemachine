# Distribution

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Two executables on Windows

**Three programs ship as a pair on Windows**: `karaokemachine.exe`, `km-package-builder.exe` and
`km-remote.exe` are GUI-subsystem and open a window; `karaokemachine-console.exe`,
`km-package-builder-console.exe` and `km-remote-console.exe` are console-subsystem, open no window,
and are **debugging tools** — what to run when you need to read why something did not start.

**Only Windows gets two, because only Windows has a subsystem to choose.** A macOS `.app` already
has no terminal and a bare executable run from Terminal still prints; Linux never builds the window
at all.

**A GUI subsystem is the only thing that removes the console window that flashes.** Explorer
allocates a console before `main` runs, so a double-click blinks a black window on its way to the
real one; freeing an unwanted console (see `Unsafe code, once`) removes the one that *stays*, not
the one that *flashes*. `#![windows_subsystem]` is a property of a binary crate root, so each crate
is a library with two three-line binaries hanging off it — which is also what makes the second
executable nearly free.

**The console twin is not a lesser build**: it is the same library, and what it has that the other
cannot is the ability to answer `--help`, print a version, and hand a script an exit status it can
read. Why a program refused to start is the package builder's own answer now — a corpus it cannot
open goes on its Open page, everything earlier into a window of its own, and either way into the log
first. The machine and the remote still return a startup failure into a process with nothing to print
it, and for those two the twin remains the only way to read one. What it gives in every case is the
whole of that output as text, in the order it happened.

**The executable says which of the pair it is; nothing else can.** The rule that frees an unwanted
console reads the console's process count, which says *nobody typed this* — and that is exactly
backwards for the one executable of each pair whose purpose is to be read. Nothing observable
separates the two halves: both can be double-clicked, both are handed the same console, and the
subsystem is a flag in a header the process would have to read about itself. So each carries a
`Shell` saying which it is, and a double-clicked twin prints its address rather than opening a
browser tab.

**The tools' twins are gated on `required-features = ["desktop"]`; the machine's is not.** Every
build of `karaokemachine` has a window. A `--no-desktop` or Linux build of either tool has no
window and the plain executable already *is* the one that prints, so shipping a second copy of it
under another name would be a choice nobody could make correctly.

**`--register` associates `.kmbuild` with the *windowed* package builder**, since a corpus
double-clicked in a file manager should open a window rather than a browser tab. The remote has no
`--register`, having no document type to claim. `tools/dist/cmd.sh` stages
`KM Package Builder.app` beside the tool's folder on macOS — beside and not instead of,
because the folder is still the command line and a `.app` cannot be typed.

## The machine's console window

**There isn't one, and the twin beside it is named in no end-user document.** `README.md` does not
mention `karaokemachine-console.exe` at all, and the staged `README.txt` of the Windows folder gives
it one line saying what it is for.

**What an end user reaches for is `--show-paths` and `--set-password`, and the plain name answers
both.** Standard handles are inherited whatever the subsystem — see
`crates/platform/km-console/src/lib.rs` — so `karaokemachine --version | cat` writes down a real
pipe, and `dist_version` in `tools/dist/common.sh` stages every release on exactly that. What the
twin buys is a shell that waits and output that does not arrive after the next prompt: a convenience
for reading at an interactive prompt, not the way to reach the command line.

**The machine is a fullscreen appliance under a television**, so a console here is not a terminal
somebody occasionally reads but a black window sitting beside the lyrics for the whole evening. A
staged `README.txt` warning people to expect it would be the tell — a feature nobody has to be
warned about is a feature.

Two ways this differs from the package builder's pair. **The subsystem is `cfg(windows)` with no
feature under it**, because there is no build of this executable that wants a console — that tool
has browser-served builds with no window, one of which must keep talking to whoever started it,
while `--headless` here is a way of *running* the machine and not a build of it. And **the twin has
no `required-features`**: both binaries are built on every platform and only Windows *stages* both,
which keeps `cargo build --workspace` producing exactly what the Windows folder ships. macOS and
Linux need neither half.

## Bundling assets

**Anything a binary needs in order to work is compiled into it wherever that is possible.** A file
that has to travel beside an executable is a file that can be left behind by a copy, and its absence
is never a clean failure — an unreachable stylesheet or script leaves a page that renders and does
nothing, with no error to search for. `km-package-builder` is the finished shape: one executable, no
folder, nothing to lose.

**The exceptions are by size and license, not by convenience.** The 31 MiB GeneralUser GS SoundFont
stays a file, fetched by `tools/setup/fetch-assets.sh` rather than committed, because embedding it
would put tens of megabytes into every build of every crate that links the engine. The wallpapers
stay files for a second reason that is not about size: a Pixabay or Pexels pack is **not** a
redistributable, which is why the shipped set is CC0 and a built pack is neither committed nor
staged. See `Where a wallpaper pack's photographs may come from`. A new asset is embedded unless it
can be argued into one of those two exceptions.

**Nothing is ever fetched from the network at run time to make the machine work**: a machine with a
corpus on it may have no internet, and a CDN that cannot be reached is the same inert page by another
route. **The qualifier is doing real work** — a SoundFont bank can be fetched at run time on an
explicit request, see [`Nothing downloads`](repository.md#nothing-downloads) — and that does not
touch this row: the bank the machine *needs* is bundled, every page and asset it renders is local,
and a machine that never reaches the network is missing nothing it was shipped with.

## What the machine *is*, on Linux

**A Debian appliance with no desktop on it.** The supported Linux deployment is a box under a
television running the shipped `.deb`, coming up on boot as a system user `karaoke`, with the screen
driven straight from a bare virtual terminal — SDL3's `kmsdrm` backend on DRM/KMS, no X server, no
Wayland compositor, no display manager, no session to log into.

This is a decision about what the product is and not only how it is built: everything a desktop
would provide here is something that can fail, start slowly, or draw over the lyrics, and none of it
is wanted on a machine whose entire interface is a television and the phones in the room. `cage` — a
compositor that runs one fullscreen application and nothing else — is the documented fallback for
hardware where kmsdrm will not work, written into the shipped unit file as two commented lines.

The package stays a perfectly ordinary desktop application otherwise: it ships the unit **disabled**,
so installing it gives a menu entry and nothing seizes the screen. `tools/platform/linux/deploy.sh`
is what enables it, and enabling it is the step that turns a computer into an appliance.

## An appliance install carries no display-server stack

**A box under a television opens no X connection ever, and installing the machine on one leaves it
unable to.** That is a requirement rather than a side effect: everything a desktop would provide here
is something that can fail or start slowly, and a library the machine will never call is one more
thing the box carries, updates and is scanned for.

**`libX11` itself is the exception, and it is not ours to decline.** Mesa's EGL and gallium packages
hard-depend on `libx11-xcb1`, and `kmsdrm` draws through Mesa — so `libX11` and the `libxcb` family
are on any box that renders at all. What is absent is the part that would let anything *use* them:
SDL's x11 driver opens `libXext`, `libXrandr`, `libXcursor`, `libXi`, `libXfixes`, `libXss`,
`libXtst` and `libXrender` by soname, and without those it cannot start whatever else is present.
That is the line the verifier asserts, because it is the one that means something.

**One package serves both, and apt chooses which deployment it is.** SDL reaches every video backend
with `dlopen`, so the X11 and Wayland libraries are `Recommends` and nothing else has to change: a
plain `apt-get install` takes them and a desktop gets its backend, `--no-install-recommends` takes
none and the box runs `kmsdrm`, and `deploy.sh` passes that flag. What a desktop declining Recommends
gets is a machine that starts and finds no video device, which is non-fatal and logged — the honest
price of letting apt make the choice at all.

**The package carries its own ffmpeg, and without that none of the above is reachable.** Debian's
`libavutil` has `libX11` as a direct `NEEDED`, beside `libva-x11`, `libvdpau` and `libOpenCL`, so a
package linking it loads an X client library and two video-acceleration stacks into the process on a
box with no X server — whatever its dependency list says. The LGPL build this repository already
makes for the tarball links none of them, and `tools/platform/linux/ffmpeg-lgpl.sh` refuses its own
output unless that stays true. `libopenh264` travels with it, `libavcodec` linking it to encode
H.264.

**What that trades away is hardware decode, and nothing is using it.** `km-video` decodes in software
on every core and asks for no hardware device, so Debian's acceleration libraries were loaded and
never called. Should it ever want them, VA-API through its **DRM** backend is X-free — `libva2` and
`libva-drm2` reach only `libc6`, `libdrm2` and each other — so this decision forecloses nothing; it
is **VDPAU** and VA-API's X11 backend that cannot be had without X.

**`deb.sh --system-ffmpeg` builds the other trade**, linking the distribution's ffmpeg and accepting
X on the box. Vendored libraries are what Debian policy tells a packager not to ship, so that is the
build for anybody rebuilding this for a distribution, and it is not what a release carries.

## The three tools are a package of their own, which the machine recommends

**`karaokemachine-tools` carries `km-package-builder`, `km-remote` and `km-admin`, and
`karaokemachine` Recommends it.** Linux was the one platform where the tools could not be installed
at all — Windows has a setup program and macOS one `.pkg` for every product — and a folder to unpack
is not an answer on a system with a package manager.

**Recommends rather than Depends, and that is the same mechanism the backends use.** A desktop
running `apt-get install karaokemachine` gets the tools; `--no-install-recommends`, which
`deploy.sh` passes, leaves a television box without 35 MB of curation tools nobody will run on it.
One flag decides both what a box carries and what it is for.

**It is a release asset of its own, and the page names both files in one command.** A Recommends is
satisfied out of a repository, and there is none: the two packages arrive as two downloads, so
`apt` is given both paths at once and resolves the relationship between the files in front of it.
A reader wanting the machine alone names the one file, which is the same choice
`--no-install-recommends` makes on a box that already has a repository.

**They install under `/opt/karaokemachine/tools/` and Depend on the machine, for one directory.**
`km-package-builder` is built with `video`, so it links the four ffmpeg libraries the machine's
package already carries at `/opt/karaokemachine/lib`; an rpath of `$ORIGIN/../lib` reaches them.
A second copy would be 20 MB saying the same thing twice, and the alternative — a third package
holding only libraries — costs more than it returns while there are two consumers.

**None of them has a window here**, so the package installs three servers and three names on `PATH`.
What each opens is a browser, which is [`The package builder's
window`](curation.md#the-package-builders-window) and its two siblings unchanged.

## What the box shows before the machine does

**The boot is part of the appliance, and it is ours from power-on.** Everything from the machine's
first frame was already designed; the seconds before it were Debian's — a bootloader menu with a
distributor's name on it, then kernel text, then black. On a box under a television that is the one
stretch that still looks like a computer starting up, and it is the stretch somebody watches while
wondering whether the thing is broken.

Two decisions, and they are separate because they can fail separately.

**The bootloader menu is hidden behind a key, not removed.** `GRUB_TIMEOUT_STYLE=hidden` with a
one-second timeout draws nothing at all and brings the menu up on any keypress. **Zero seconds is
the wrong answer and the reason is the firmware**: GRUB's `keystatus` — the held-SHIFT test that
lets a BIOS box show its menu with no timeout at all — does not exist under EFI. So on a UEFI box a
zero timeout is not "faster", it is *no way into the menu short of a live USB*, on hardware with no
keyboard normally plugged into it. A window there has to be. The number is fixed rather than a flag,
because a flag would be a second place to argue it.

**One second rather than three, and a television is what settled it.** The window is not the only
dead time before the mark: the panel drops the HDMI link for about two seconds when i915 takes the
connector over from the firmware, and the window stacks straight on top of that. Three seconds
measured as five seconds of nothing, which is long enough to read as a machine that has not started.
One second still catches a key *held down* from the moment the box is switched on, which is how
anybody actually reaches a boot menu — the three-second version was buying a comfortable window for
a keypress nobody makes casually.

Debian's `recordfail` handling is deliberately untouched: a boot that failed brings the menu back by
itself, and that safety net is part of what makes hiding it defensible.

**The splash is the machine's own mark on the machine's own ground.** A Plymouth theme drawing
`Theme::background` — the same near-black the idle screen uses — so the handover from splash to
application is not a change of picture but the mark going away. The mark is sized as a fraction of
the panel rather than shipped at a size, because a fixed image is three different designs across a
720p set, an ultrawide and a 4K television.

**And the splash goes when the machine is ready, not when the boot target is.** This is the part
that had to be learned on a television, in three passes.

Debian quits Plymouth at `multi-user.target`, which on the appliance is about four seconds before the
machine has a first frame — the SoundFont, the catalog and the packages folder all load after it. So
the mark went, and four seconds of *something else* filled the gap: first systemd's job status on the
restored text console, then, once that was silenced, plain black. **A splash that leaves before there
is anything to replace it is worse than no splash**, because a picture that disappears reads as a
crash where a blank screen reads as a wait.

So the ordering is reversed: `plymouth-quit` is ordered **after** the machine, and the machine's unit
is `Type=notify` — it says it is ready once its audio, catalog and API are up and one line before it
starts the display. The splash covers the whole load and goes at the last possible moment.

**Ready therefore means everything except the screen**, and it has to: the display cannot start until
Plymouth releases the graphics device, and Plymouth will not release it until the machine says it is
ready. That is not a compromise — a television switched on at nine in the evening is a state this
machine is built to sit in and keep retrying, so "ready" was never going to be able to mean "there is
a picture".

Three smaller things belong to the same decision, because each is invisible on its own and each
undoes the others by its absence: the quit retains the last frame, so it survives the second the
display takes to appear; the kernel command line silences systemd's status, because a retained frame
is still a framebuffer and console text draws over one; and the machine's unit no longer resets its
virtual terminal, because resetting one repaints the console.

**The bootloader's own text is hidden too, and not by editing the bootloader's own file.** GRUB
prints `Loading Linux …` while it reads the kernel and the initramfs, and `quiet` covers the kernel
and says nothing about the loader — so on a real boot those two lines own the television for longer
than anything else, the menu included. Debian's generator can suppress them and hardcodes the flag
off, in a dpkg conffile; editing that would trade a few seconds of text for a package prompt on a box
with no keyboard, which is the worse of the two. A file in `/etc/grub.d`, which exists to be added
to, makes the loader's text black on black instead. The menu keeps its own colours, so a keypress
still shows something readable.

**Both ship inert, on exactly the terms the systemd unit ships disabled.** The package carries the
theme and selects nothing; a theme directory on disk changes no boot, in the same way a unit file on
disk seizes no tty. Selecting it — and hiding the menu — is
`tools/platform/linux/appliance-boot.sh`, run once per box, which is the same act as
`deploy.sh` enabling the unit. Plymouth itself is a `Suggests`: pulling a boot splash and an
initramfs rebuild onto somebody's desktop as a side effect of installing a karaoke player would be
the overreach the disabled unit exists to avoid.

**And it is a separate script rather than a flag on the deploy**, because a deploy runs on every
commit and must stay unattended, while this rewrites a bootloader's configuration and rebuilds an
initramfs. It is also the one piece of the appliance tooling that asks for a password: a box locked
down to the specific commands a deploy runs does not permit `update-grub`, and should not.

Everything it changes is written aside first, exactly once, and `--revert` puts the box back. A box
that cannot be put back is not configured, it is modified.

## Any screen size, and the screen chooses the mode

**Every size and every shape is supported, and no resolution is privileged.** Nothing stores a
design resolution or a target aspect ratio. Sizes come from the live drawing surface every frame,
type scales from height and margins from width, panels are fractions, and pictures are letterboxed
rather than cropped — so a layout is derived for the screen in front of it rather than fitted to one
screen and scaled for the rest.

**The display chooses the mode, not the machine.** The window is created at the connector's
preferred mode before it goes fullscreen, which is what a screen is asking for when it advertises
one. There is no resolution setting and no mode flag, and adding either would be answering a
question the display has already answered better.

**16:9 is the common case and not an assumption.** DCI 4K is 4096×2160 where UHD is 3840×2160, so
two sets both sold as 4K differ in width alone; an ultrawide is wider again and a phone held upright
is taller than it is wide. A layout right about one of those can be wrong about the next, which is
why none of them is the one the geometry is tuned to.

**The test matrix is coverage, not a list of targets.** It spans 0.45 to 2.37 in aspect ratio
because the cheapest way to keep a layout honest about shape is to deny it a single shape to be
right about. Those sizes are standard formats chosen for their shape, and none of them is a claim
about the screen anybody is watching.

A box under a television is what the appliance is for, and the sets it meets there are the reason
the range is worth testing — but that is where this decision is *exercised*, not what it is limited
to.

## What a shipped build says out loud

**Plain `info`, and diagnostics are asked for by name.** A release must not produce developer output
by default, which decides on the owner's behalf that they wanted it. `karaokemachine -v` gives this
crate's debug stream, `-vv` gives everything, and `RUST_LOG` wins over both when a question needs
something the ladder does not offer. **Every shipped binary carries the same ladder** — the machine,
the offline remote and the package builder.

### `logging.level` is the rung below the flag

**A settings key saying what `RUST_LOG` would say, because the machine that most needs the level
changed is the one with no command line and no unit file.** A flag reaches the run somebody types
and a variable reaches the unit somebody wrote; a box under a television that a person walks up to
and starts by double-clicking its icon has neither, and it is at once the machine most likely to
fail with nobody watching and the one hardest to make fail again on purpose.

**It takes a directive rather than a rung, and that is the whole of why it is worth having.** A rung
is a fixed pair of names chosen per program — this crate at `debug`, everything at `trace` — and the
question somebody has when they go looking is usually narrower than either: the HTTP layer and
nothing else. `RUST_LOG`'s grammar answers that and a number cannot, so the key speaks it, a bare
`"debug"` reads as the level it looks like, and the key **replaces** the ladder exactly as
`RUST_LOG` does rather than being folded into it. Folding would mean deciding whose quietened
dependencies survive somebody else's directive, which is a question with no honest answer.

**`RUST_LOG`, then `-v`, then the file.** `RUST_LOG` keeps winning outright, which is the exemption
this decision already records; below it the order is the one every other setting here uses, so one
run typed at a keyboard is never an edit to the machine.

**A directive nobody can read leaves the ladder standing and says so**, exactly as a bad `keep` does
— reported once there is a subscriber to report it through, which is necessarily after the filter it
failed to set. Guessing would be the worse failure: a typo that quietly became `info` is a machine
somebody believes they turned up, and they find that out on the evening they go looking for the run
that broke.

**Whether to run a measurement is not a question about log levels.** A level says how much detail you
want about what the program is doing; it should not also be the switch deciding whether the program
*measures its own frame rate*. So the frame meter is behind `--frame-stats` (or `KM_FRAME_STATS=1`,
for Android and for a systemd unit, neither of which can pass an argument), is not constructed at all
when it is off, and reports at `info` when it is on — having asked for it by name, you should not
then have to work out which level it hides behind.

**A route in that goes through the environment is not a route on Android.** Setting an environment
variable for an app requires the `wrap.<package>` system property, and a retail Android TV's SELinux
policy refuses `adb shell` the whole `wrap.` class, short names included. So on the one platform
`KM_FRAME_STATS` exists for, there is no way to set it, and measuring frame timing there means a
throwaway build with the flag forced. **The fix will have to be the shape the debugging switch
took**: something the API can reach, so a machine with no command line can be told. Not done. What
makes it survivable rather than urgent is that the fault it would be used to find reports itself
without any flag; see `A song that stopped says so without being asked` in
[`song-sources.md`](song-sources.md).

**The appliance unit pins no filter of its own**, which would only re-enable what this turned off.

**Crates measured to be chatty are named and held down**, which is this same decision applied to
somebody else's library: `symphonia`'s MP3 demuxer announces itself at `info` on every MP3+G pair
opened, and `info` is a level no verbosity change was going to reach.

**How far down follows how many files the program opens.** The machine holds them to `warn`: it opens
a file when somebody sings, so a warning is one line and belongs in the journal beside the song it is
about. The package builder holds them to `error`, because a scan opens every file in a folder and that
crate speaks at `warn` as well as at `info` — a tag whose CRC disagrees is a line per file, across a
whole corpus, written over a meter that rewrites one line and cannot know anything else reached the
terminal. Nothing is lost by not printing it: a file that will not read is recorded as a scan status
and listed on the failures page, which is a channel the tool controls rather than one a dependency
decides.

## What a staging run says out loud

**Quiet by default, `-v` to watch, and every step replays what it held back if it fails.** This is
`What a shipped build says out loud` applied to the tooling that produces those builds, where the
noise is somebody else's: `cargo build --release`, which `tools/dist/cmd.sh` with no arguments runs
six times. A full Windows release prints thirteen lines.

**What stays is what the run produced** — the `== phase` headers, the `staged <path>` report and its
byte totals, every `verified:` line, every warning, and the runnable trailer. What goes is the build
logs, and the detail lines that name *which* ffmpeg or libclang a build linked, which is a question
you go looking for rather than one you want answered every time.

**Two mechanisms.** Cargo takes its own `--quiet`, because that flag drops `Compiling`/`Finished`
and leaves every rustc diagnostic exactly where it was, so a quiet build that fails is already as
diagnosable as a loud one. Everything else — `docker build`, `cargo deb`, `fetch-assets.sh` — goes
through a helper that captures and replays the whole log, because none of them separates its report
from its noise. A silent minute is answered by a `built in 2m14s` rather than by a scroll.

`DIST_VERBOSE` crosses into the Debian container as an environment variable, since the in-container
scripts have no argv at all; and `docker run` itself is deliberately *not* captured, because what the
container prints is the report and only cargo's stream inside it is a log.

## Running what was staged does not wait

**`task run` starts the staged machine and hands the prompt back; `WAIT=1` blocks.** What is being
launched is a karaoke machine, which somebody uses for an evening — so blocking means staging a build
and running it costs the terminal until the singing stops. `WAIT=1` is `exec`, so Ctrl-C reaches the
app and its exit code becomes the task's.

**`CONSOLE=1` on Windows also blocks**: asking for the twin that prints means wanting to read what
it prints, and detaching it would print into a console nobody is looking at.

Output from a detached run is discarded — one that will not start is diagnosed by running it again
with `WAIT=1`, which is cheaper than a log file somebody has to remember to delete.

**A bare `&` is not one of the options**, which is the trap: Task's embedded shell waits for its
background jobs, so the launcher has to be something that forks and exits by itself — `cmd /c start`
on Windows, `/bin/sh -c '… &'` on Unix, and `open -a` for the macOS bundle, which also gives it
activation and a Dock icon. The staged-folder lookup runs either way, so a missing build still fails
with the same three lines rather than detaching into nothing.

## A second Linux carrier

**A portable folder, and a `.tar.gz` of it, beside the `.deb`.** `What the machine is, on Linux` is
still the answer to what the *product* is: the supported deployment is a Debian appliance under a
television, installed from the package, and only the package has a systemd unit, a system user and
an uninstall the system remembers.

This is the other thing people ask a Linux release for, and it is a thing a `.deb` cannot be — a
folder you unpack in your home directory, run, and delete, with no root, no package manager, and no
opinion about which distribution you chose.

It is the **same layout as the Windows folder** and that is deliberate twice over: it needs no code
change, because `Paths::discover_asset_dir` already takes the directory beside the executable when it
has an `assets` child, and it means the two portable builds are recognizably one thing rather than
two conventions. `install.sh` inside it does what one user may do without root; anything more than
that is the package's job and its header says so. **The tarball is not an appliance and does not try
to be.**

### `install.sh` is `--register`, and the desktop entry is the packaged one

**What one user may do without root is a menu entry, the icons and the `.kmpkg` type, and those are
one act rather than three.** A desktop entry carries a `MimeType=` line saying which types the
program opens, and a desktop only reads that line for a type it has been told exists. Writing the
entry without the definition is therefore not two thirds of the job: it is a claim nothing ever
looks at, and its absence is silent — every file lands, every command succeeds, and a `.kmpkg`
stays a nameless icon that opens nothing.

**So `install.sh` runs the machine's own `--register` and writes nothing itself.** The two were
separate implementations of one job, which is how one of them came to be missing a third of it, and
they disagreed about the rest: the shell copied the entry this repository ships and the binary wrote
a shorter one of its own, so whichever ran last won. The entry that lost was the better one —
`StartupWMClass` is what ties a running window back to its icon, and the hand-written copy had no
such line.

**The shipped entry and the shipped definition are compiled into the binary**, so the `.deb` and
`--register` install the same bytes and only `Exec=` differs. The package has a `/usr/bin` symlink
to name; everything else has an executable in a folder somebody chose, which is rewritten on the way
out along with a `TryExec` so a desktop hides an entry whose folder has gone.

**The tarball therefore stages no `share/`.** It existed for the copy `install.sh` used to make.

## The tarball's ffmpeg is built, not borrowed

**An LGPL ffmpeg, compiled from the release Debian itself packages, with every external library
switched off.** `Video in a release build` says Linux is the platform where video costs the shipping
story nothing, because the `.deb` names four `Depends` and Debian supplies them — but that argument
is about *naming*, and a folder cannot name anything.

**Copying Debian's libraries in instead is not open to us.** Debian builds ffmpeg with
`--enable-gpl` and says so in its own copyright file, this workspace is `MIT OR Apache-2.0`, and
Apache-2.0 is not compatible with GPL-2, so the result would be a combined work this project cannot
redistribute. It would also be enormous — measured, not guessed: 93 shared libraries and 97 MiB,
because x264, x265 and libjxl drag harfbuzz, fontconfig, pango, cairo and glib in behind them, and a
second glib underneath somebody's own GL stack is how an application starts everywhere except where
it is run.

So `tools/platform/linux/ffmpeg-lgpl.sh` builds ffmpeg 7.1.5 — **the version Debian trixie ships**,
so both Linux carriers decode through the same code — with
`--disable-gpl --disable-nonfree --disable-version3 --disable-autodetect`. **That last flag is the
whole trick and the rule is worth stating as a rule: every decoder ffmpeg implements itself, and none
that needs a third-party library.** Not a hand-picked codec list, which would have to be revisited
each time somebody auditioned a file it did not anticipate. The result is LGPL-2.1+ with no GPL
component and a fifth the size, and it links libc, libm, zlib, and the one encoder
`A streamed screen carries its own H.264 encoder` accounts for.

It costs the machine nothing, because `The packaging profile for a video` already settled that what
it ever has to decode is H.264 and AAC in MP4, all of it native ffmpeg code; the wider native set is
there so `--play ./clip.mp4` can still audition the VP9 file somebody is deciding whether to package.

**The same pin, and the same argument, as `Video in a macOS release`**, reached from Homebrew's GPL
build rather than Debian's and landing on the identical version, checksum and `--disable-autodetect`
line. Two carriers, two package managers, one conclusion.

**It is one definition and not two.** `tools/setup/ffmpeg-pin.sh` holds the release, the checksum and
the configure line; `tools/setup/fetch-ffmpeg.sh` and `tools/platform/linux/ffmpeg-lgpl.sh` source it
and keep only what is genuinely theirs — where it caches, how it reports, and, on Linux, the stamp
and the `DT_NEEDED` check. **What is shared is the definition, not the procedure**, deliberately: the
two environments really do differ, and folding the build itself together would mean restructuring a
macOS path that cannot be run from the machine this is usually developed on. The duplication is not
hypothetical: the two drifted by one flag, `--enable-zlib`, so a `.mov` with a deflated header played
out of the bundle and not out of the tarball. That is settled in zlib's favor — several demuxers want
it, `libz` is on every Mac and every Linux, and it costs the closure nothing worth counting.

## A streamed screen carries its own H.264 encoder

**openh264 travels with the machine on every platform that ships one, and nothing asks the person
running it to fetch an encoder.** A machine started with `--stream` has to produce H.264, because
that is what a television, a browser and every player in between can be relied on to play. ffmpeg
implements no H.264 encoder itself, so one external library is linked: `--enable-libopenh264` in the
shared pin, which is BSD-2 and, unlike x264, needs no `--enable-gpl` — so the LGPL posture the pin
exists to protect is untouched.

**Which encoder a build carries is not something a setting can know beforehand**, and that is why
`stream.encoder` defaults to `auto`. An ffmpeg built here carries openh264 and nothing else; a
distribution's is built `--enable-gpl` and carries x264 and no openh264 at all. `auto` takes
whichever of the two is present, so a machine streams out of the package, the tarball, the Windows
folder and the macOS bundle without anybody knowing which library answered. A name written down is
used exactly as written and reported if it is absent, which is what keeps a hardware encoder
measurable.

**The tarball carries the library rather than depending on it**, the same rule the four libav* files
beside it follow: a folder can name nothing, so anything it needs it holds. The BSD-2 text travels in
`LICENSES/`. libstdc++ is the one thing left to the system, openh264 being C++ where the rest is C —
a bundled libstdc++ older than the system's breaks every C++ object loaded after it, and every
distribution the tarball claims already has one.

**The patent position is understood and accepted.** Cisco pays the AVC/H.264 patent pool for binaries
Cisco itself distributes, and that payment does not follow a build made from source — which is why
Debian ships `libopenh264-cisco6`, fetched from Cisco at install time, beside the package built in
its archive. What ships here is built from source, on all three platforms alike, and a home karaoke
machine playing to a television in the next room is the use this is judged against. Anyone
redistributing this commercially is the party that has to weigh it again.

**The alternative was a machine that cannot stream at all**, since a folder cannot fetch a library at
install time and a stream with no encoder is a mode that starts and then stops. Between an encoder
that is present and a feature that is absent, the encoder is the one somebody asked for.


## A font, in the tarball only

**The tarball stages DejaVu Sans as `assets/fonts/karaoke.ttf`; the package goes on naming
`fonts-dejavu-core`.** `km-display` looks for a bundled font at exactly that path. The `.deb` has no
need of one, since it can name a font package and Debian's is the first entry in the hard-coded list
of system paths anyway.

Neither is true of a folder: it can name nothing, and that list is Debian- and Arch-shaped, so on
Fedora, where DejaVu sits under `/usr/share/fonts/dejavu-sans-fonts/`, none of the candidates match,
`find_font` returns `None`, and km-app treats a display that will not start as non-fatal — a black
television with the reason in a log nobody is reading.

This does **not** reopen `Bundling assets`, which is about what the repository ships: nothing is
added to `assets/` here and no build but this one gains a file. The font is staged by the carrier
that cannot express a dependency, exactly as its ffmpeg is. `display.font` still overrides it.

## A CJK face is borrowed, never bundled, and opened only when asked for

**Nothing ships a CJK font, on any carrier.** `km-display` holds a list of the places each system
keeps one — MS Gothic and MS JhengHei on Windows, Hiragino and Arial Unicode on macOS, Noto CJK on
Linux and Android — takes the first two that exist **and answer for different scripts**, and stands
them behind whichever font [`A font, in the tarball only`](#a-font-in-the-tarball-only) chose. A path
that is not there is skipped, exactly as the Latin list's are.

**Bundling one would cost every carrier 20 MB for 0.3% of a corpus.** That is the measurement: 0.3%
of the `.kar` files are Shift-JIS or Big5. Noto Sans CJK is larger than the SoundFont that
[`Bundling assets`](#bundling-assets) already argues into a file rather than the binary, and it would
go into the `.deb`, the tarball, the Windows folder, the macOS bundle and the APK alike — where three
of those five platforms already have the glyphs sitting on disk.

**Linux needs three entries for one package, and that is measured rather than assumed.**
`fonts-noto-cjk` on Debian, `noto-fonts-cjk` on Arch and `google-noto-sans-cjk-fonts` on Fedora
install the same `NotoSansCJK-Regular.ttc` under `/usr/share/fonts/opentype/noto/`,
`/usr/share/fonts/noto-cjk/` and `/usr/share/fonts/google-noto-sans-cjk-fonts/` respectively — three
unrelated directories, of which two were guessed wrong before a container was asked. That is the same
shape as the Fedora hole in the Latin list [`A font, in the tarball only`](#a-font-in-the-tarball-only)
already describes, caught this time before it shipped.

**The `.deb` does not gain a dependency on a CJK font package**, where for the Latin font it names
`fonts-dejavu-core`. The two are not alike: a machine with no Latin font shows nothing at all, and one
with no CJK font shows everything except 0.3% of a corpus. Tens of megabytes on every install is the
wrong price for that.

**A path taken from documentation instead of off a machine is a path that may not exist.**
`PingFang.ttc` names a font that is installed and a file that is not there: macOS 26 keeps PingFang
inside `FontServices.framework`, which is not a path anything should depend on. Behind the cap of
two that single absence is enough to leave **Simplified Chinese with no face at all**, because the
entry carrying it is fourth in a list that stops at the second, while `has_cjk()` reports true
throughout. `Supplemental/Arial Unicode.ttf` takes the second slot instead, chosen by reading the
`cmap` of face 0 of every candidate rather than by the language in its name: it is the only one of
them carrying both Chinese variants and Hangul, so two files cover what four language-shaped ones
did not.

**Windows fails the same way from the opposite cause.** All five of its entries are present on a
stock install, so taking the first two takes MS Gothic and Yu Gothic — both Japanese — and Chinese
and Korean are never reached: on macOS every path is tried and one is missing, here none is missing
and the cap runs out. So the list carries, per file, **which of the four scripts it answers for** —
Kana, Simplified, Traditional, Hangul — and a file is taken only when it answers for something
nothing already taken does. The two slots then hold two scripts by construction rather than by the
order somebody typed, and a face that answers all four, which is what Noto CJK is, ends the search
alone instead of being joined by six more faces buying no glyph.

**The masks are the conservative reading, and the ordering still matters.** Under-claiming costs one
more candidate looked at; over-claiming leaves a script with nothing and no message. And the rule
stops a slot going twice to one script — it does not choose *which* script goes first, which is still
the corpus: Japanese, then Chinese. Putting Yu Gothic below the Chinese entries, so it looks like a
spare, gives a Windows box without MS Gothic two Chinese faces and no Japanese; the test that says so
is in `km-display`, and it checks both platforms without opening a font.

`display.font_cjk` in `settings.json` still covers whatever any platform's list misses — the same
shape `display.font` already has, and for the same reason. It cannot be the *plan* for a platform:
nobody sets a setting for a fault they cannot see, and a skipped path looks exactly like a system
with no CJK font on it.

**Opened on demand rather than at start.** A `TTF_Font` is bound to its point size and the display
holds six sizes, so each fallback file is six more open faces over 8 to 20 MB. The machine that pays
that should be the one that has actually been handed a Japanese title, and the reason it can be
deferred at all is that the display already knows how to rebuild its fonts — a resize makes it do the
same thing. Why it must be a *rebuild* rather than an attachment is in
[`Non-Latin text`](songs.md#non-latin-text).

## A folder with everything in it

**`tools/dist/bin.sh` stages `dist/bin/<platform>` and `dist/bin-console/<platform>`, each holding
every executable this platform can build.** `bin/` takes the windowed form of anything that has one
and the plain form of the rest; `bin-console/` takes the console form, and the same plain rest.

**A macOS `.app` is a windowed form, and the bare executable staged beside it is the console one** —
the same pairing Windows spells as `km-remote.exe` beside `km-remote-console.exe`, written the way
macOS spells it.

**The two are symmetrical in shape and not in purpose.** Same three rules, same gather, same zip —
but `bin/` is the release in one folder and `bin-console/` is the copy to reach for when something
will not start. `tools/dev/soundfont.sh` drives the machine out of `dist/bin-console/<platform>` and
nowhere else, precisely because it wants to read the answer.

Executables only — no `.deb`, no installer, no tarball — so on Linux it gathers the portable *folder*
and never the package. The per-product folders are untouched and stay the answer to what a release
*is*; this answers the different question people actually ask, which is *give me one folder with all
of it in it*. The alternative is unzipping seven folders and merging them by hand, and there are
exactly two places that merge goes wrong: the ffmpeg libraries appear in five of them, and on Windows
three products ship a windowed executable and a console twin under names that differ.

**It gathers rather than builds.** It runs the same staging scripts `task dist` runs and copies what
they produced, so not one fact about which crate takes `video`, which takes `desktop`, which four
DLLs get staged, how a macOS load command is rewritten or what a README says is written down a
second time. `--no-build` gathers what is already there and builds nothing.

**Its own knowledge is three rules about shape, not a table of products**: a `*.app` beside a staged
folder is a windowed form, a file named `<x>-console` is a console form and its `<x>` sibling is
therefore the windowed one, and everything else is single-form and goes in both. So an eighth product
needs no edit there, where a table would need one — the same argument `dist_dir()` makes about the
layout itself.

**Anything matching no rule is reported by name rather than dropped**, because a carrier that quietly
loses a file is the one failure this must not have, and the case is real:
`km-package-builder.exe.WebView2/` turns up in that tool's staged folder the first time anybody runs
the exe from it. **A collision on identical bytes is a no-op and a collision on different bytes is an
error** naming both — the four DLLs arrive five times over, which is a claim worth one `cmp` rather
than an assumption.

**What `--no-video` means to a gather is answered by looking in the folder rather than by reading the
flag.** The marker goes on the *declined* build, so a product with no such feature to decline never
carries one, and any rule that has to accept an unmarked folder accepts a video build with it.
Whether the libraries are actually sitting there is the thing that is true or false, so that is what
is asked — and it decides both the refusal and the sentence the generated README ends with, so that
README cannot lie about a folder whatever flags produced it.

**The folders carry no version and the archives do.** A folder is where you keep the current build,
in the way `KaraokeMachine.app` is; a number belongs on the thing you hand over. The cost:
`clean:old` can never remove a versionless folder, exactly as it cannot remove a bundle, so `--all`
is what does (it does sweep the zips), and on Windows and macOS the two folders are are a second
copy of a 31 MiB instrument bank. On Linux they are byte-identical, because nothing has two forms
there, and the report says so rather than leaving it to look like a bug.

## A Windows setup program

**One installer for all seven products, built with Inno Setup 6, installing per-user into
`%LOCALAPPDATA%\Programs` with no elevation.** Every other carrier is something you unpack, and
`A folder with everything in it` answers *give me all of it*, not *I double-clicked setup.exe*. What
a Windows recipient gets without one is 188 MB they must not move files out of, seven executables and
a second folder holding the four console forms, no Start Menu entry, and no `.kmbuild` association
unless they found `--register` in a README.

**Components rather than seven installers**, because the products are used together and the shared
payload is most of the bytes: the instrument bank and the four ffmpeg DLLs are attached to the
components that read them, so somebody who wants only the remote pays for neither.

**One product is also handed to somebody who has no machine, and it has a carrier of its own.** The
remote is what a guest holds while somebody else's machine plays, so the computer it lands on wants
none of the rest and its owner is not choosing from a component list. `A setup program for the remote
alone` is that carrier and the argument for it; every other product reaches a Windows box through
this one.

**Per-user because everything the installer configures beyond the files is per-user**: the `.kmbuild`
association is written under `HKCU\Software\Classes` — `register.rs`'s own decision, not this one —
and the PATH entry is `HKCU\Environment`, so a machine-wide install would place files every account
can see and configure them for exactly one. It also raises no UAC prompt and matches where `winget`
puts per-user software.

**Inno rather than NSIS**: NSIS caps strings at 1024 bytes in a stock build, so editing a real
`PATH` needs a shell-out to PowerShell, its bundled downloader speaks HTTP only, and an upgrade
means reading `UninstallString` back by hand — Inno has an unbounded string type,
`DownloadTemporaryFile` over HTTPS, and recognizes its own `AppId`, so three workarounds become
three built-ins. MSIX cannot be installed unsigned at all and MSI's per-user story buys group-policy
deployment nobody asked for.

**What it deliberately does not do**: no MSI, no MSIX, no auto-update, no code signing. It is
unsigned, so SmartScreen shows *"Windows protected your PC"* on a recipient's first run and they must
click through *More info → Run anyway*. The fix is a purchased certificate, and therefore a purchase
rather than a build step.

## What an installed build contains

**The windowed executable of each product and never its console twin, always the video build, and a
README written for an installed build rather than for a folder.**

**The folder README is the wrong document for an installed build**, and the difference is not
wording: it says the folder holds every executable this platform can build, when a setup program
installs what was ticked; it lists all eight programs, because the list is a scan of the *staging*
folder baked in at build time; and it says to remove it by deleting the folder, because "nothing was
installed anywhere else and nothing was registered" — beside, on Windows, an uninstaller, a Start
Menu group, a `PATH` entry and a file association, and on macOS an uninstaller sitting in the very
same folder. So `dist_installed_readme` in `tools/dist/common.sh` writes what an installed build
gets, branching on platform because the two layouts genuinely differ — one folder and a Start Menu
against `/Applications`, `/usr/local/bin` and a `.command` you double-click — and shared between the
two installers on the same argument as the ffmpeg license note beside it: two setup programs
describing how to remove the same product differently is exactly what one copy prevents. The macOS
half ends with the same `data-locations.txt` the conclusion pane and the uninstaller print.

**Each installer names the exclusion rather than dropping the file quietly** — Windows in its
payload-coverage check, macOS in `excluded()` — because "nothing is dropped from a carrier by
accident" is what those two checks exist to enforce, and an exception that is not stated is the thing
they are for. **Both round trips read the installed README**, which is how a document contradicting
the uninstaller two folders away would otherwise survive every build.

**No console twin**, because an installed program has a Start Menu entry and a `PATH`: a program
appearing twice in a Start Menu under names differing only in a subsystem is the confusion a setup
program exists to remove. Nothing is lost — the portable folder, the zips and
`dist/bin-console/<platform>` all carry the twins, and the four command-line tools are *not* twins
and are installed as themselves.

**The video half is the same judgment**: there is no `--no-video` installer for the same reason there
is no half-sized Start Menu, because a person double-clicking setup.exe is not choosing a feature
matrix. `--no-video` still means something where it always did, in the portable folder. A staging run
that cannot find the ffmpeg libraries therefore **stops** rather than quietly producing a setup that
lists video songs and refuses to read them.

**The macOS package has no console-twin half to take, and has a mirror image of that problem.** There
the *machine* has no bare executable at all: it is staged only as a bundle, so what goes on the
`PATH` is not a second file but a two-line shim into the one inside the `.app`. The six real commands
are symlinks. Which of the two each product gets is measured in `The macOS installer` in the
architecture notes.

**On Windows it contains one thing that is not a program: a `Karaoke songs folder` entry pointing at
the packages folder.** "Where do I put my songs?" has no other answer for somebody who installed by
double-clicking a setup program — `--show-paths` needs a terminal they never open, and the idle
screen deliberately will not put a Windows path on a television. The installer makes the folder so
the entry has a target, and never removes it. **macOS needs nothing**: its closing pane and its
uninstaller both name that folder, from the one `data-locations.txt` they share, and an alias in
`/Applications` is not a thing Mac software does.

## An installed machine can be started streaming without a command line

**Every carrier offers a second launcher that runs the machine with `--stream`**, beside the one that
opens its television. A streaming run is a way of running the machine rather than a build of it — it
overrides `display.enabled` for the process and writes nothing back — so the two launchers are one
program, and an install started one way still opens its television the next.

**What earns it is that the mode is otherwise unreachable from an installed build.** Somebody who
installed by double-clicking a setup program has no terminal in the picture, and a flag they cannot
type is a feature they do not have. It is the same argument the `Karaoke songs folder` entry already
makes one section above, about a different thing a terminal would otherwise be needed for.

**Each platform spells it the way that platform spells a launcher**, and only one of the three costs
anything:

- **Windows**: a Start Menu entry carrying `--stream`. No tick box beside it — a Start Menu entry is
  free, where the desktop icon is a task because a desktop is somebody's own space.
- **Linux**: an action on the desktop entry, which is what a launcher's right-click menu is for. The
  `.deb` and `--register` ship the same file, so both carriers get it from one place.
- **macOS**: a second bundle, because a bundle carries no launch argument anywhere in its manifest.
  It holds a launch script and an icon and nothing else, and what it starts is `KaraokeMachine.app`
  beside it — the bundle, through LaunchServices, and never the binary inside it, for the reason
  [`The icon in a macOS menu bar is a silhouette, and the run behind it takes no Dock tile`](interface.md#the-icon-in-a-macos-menu-bar-is-a-silhouette-and-the-run-behind-it-takes-no-dock-tile)
  gives. So there is one machine, one set of assets and one set of ffmpeg libraries however it was
  started, and one identity while it runs. It rides in the machine's own component, a launcher that
  could be declined separately being a launcher that can point at nothing.

**The run it starts has no window and, on Windows and macOS, no console**, which is what the icon in
the bar is for — see `A running server has an icon in the bar`. On Linux there is no icon bar and the
action is still worth having: a streaming machine there is an appliance or a terminal away, and a
launcher is neither.

**Nothing is offered for the modes a person does not start deliberately.** `--headless` has no
launcher and wants none: it is what a machine falls back to when a screen will not open, and a
launcher for it would be offering somebody a machine with no output at all.

## What setup fetches

**Exactly one thing, and only when it is missing: Microsoft's WebView2 bootstrapper.** This narrows
`Nothing downloads`, and the half that matters is unchanged: nothing in the *product* reaches the
network for a song. The installer is not the product; it is the packager's side of the line that row
already draws.

It is asked only when the package builder or the remote was selected, because those are the two that
put a webview in a window, and only when the runtime's `pv` is absent from the EdgeUpdate keys.

**A failed download is a message, not a failed install**: `The package builder's window` already has
both tools falling back to the ordinary browser when a webview cannot be created, so the runtime is a
convenience and treating it as a prerequisite would make setup fail for something the program routes
around by itself. It fetches no song, no asset and no update, and there is no auto-updater to add one
later without reopening this row.

**This is a Windows-only narrowing, and the macOS package restores `Nothing downloads` whole: it
fetches nothing, ever.** There is no WebView2 analogue to fetch because `WKWebView` ships with the
operating system. The next carrier has to argue its way past this row too.

**The instrument-bank tick box does not change this, and that is why the tick box has the shape it
has.** `Offering the recommended bank at install time` adds a 261.9 MiB download to what a fresh
install ends up with and nothing at all to what setup fetches: both carriers write two lines of JSON
naming the bank, and the machine downloads it on its first start.

## Offering the recommended bank at install time

**Both setup programs offer a tick box, on by default: download the recommended instrument bank the
first time the machine starts. Neither downloads it.** What the tick box leaves behind is
`first-run-soundfont.json` beside `settings.json`, naming one bank; the machine reads it on its first
start, fetches it through the downloader it already has, and chooses it.

**The problem it solves is that the bundled bank is the one thing about a fresh install that is
measurably not the best available.** GeneralUser GS is 30.9 MiB and shipped because it can be —
the bank survey found three of fifteen redistributable, and
[`soundfont-banks.conf`](../../crates/machine/km-banks/data/soundfont-banks.conf) carries the terms
row by row. ColomboGMGS2 is
261.9 MiB, was judged the best of the sixty-eight measured, and is
[`recommended`](repository.md#which-banks-the-machine-offers) on its row. Without this it arrives
only for people who already knew instrument banks were a thing they could have an opinion about.

**Why a request rather than a download during the install.** The install stays as fast and as
offline-safe as it was; a 262 MiB file does not travel inside a 30 MiB carrier; and there is one
download path rather than three — `fetch.rs` already pins the URL, verifies the archive and the
member separately, refuses a bank the synthesizer will not open, and reports progress. Two more
implementations of that in Inno Pascal and in `sh` is exactly the drift this repository writes single
definitions to avoid.

**Ticked by default, unlike the desktop shortcut**, and the license is what makes that defensible
rather than presumptuous. ColomboGMGS2's terms are ones §8 rates *doubtful*, so
[`Where a bank may be fetched from`](repository.md#where-a-bank-may-be-fetched-from) requires them
printed beside any offer to fetch it: they are on the Windows Ready page and in the macOS choice's
description, next to the size. Somebody declining has been told what it costs; somebody accepting has
not been asked to know what a SoundFont is.

**Three refusals before anything is fetched**, and the first is the one that matters most:

- **An `audio.soundfont` already set wins**, and the request is dropped. A repair or upgrade install
  must not talk over a bank somebody chose.
- **A bank already in the folder is chosen, not fetched again.**
- **Three starts, then the request is removed** with a line on the television saying the bank can
  still be chosen from the SoundFont page. See the fourth narrowing in
  [`Nothing downloads`](repository.md#nothing-downloads).

**The size and the terms are read out of the bank table at build time**, through
`tools/setup/soundfont-banks.sh` — the shell reader that already exists — so the wording on a wizard
page cannot drift from the row it describes, and a build fails rather than shipping a tick box for a
bank that is `manual` or for a table with two recommendations.

**Not the `.deb`.** A Debian package has no tick box to offer without adding debconf, and the
appliance is the one install where somebody is at a shell anyway: `--first-run-soundfont` writes the
same request, and `task soundfont BANK=…` is there.

## The setup programs pre-write a settings file

**All three setup programs place `{"display": {"fullscreen": true}}` where there is no
`settings.json`, and that is what makes an installed machine drive a television.** The binary's own
default is *off*. See [`Windowed mode`](interface.md#windowed-mode) for the product half; this is the
mechanism.

**Because a default cannot tell an install from a checkout.** `DisplaySettings::default()` is reached
by an install with no settings file — and the other thing with no settings file is a checkout
somebody has just built. One value serving both makes the appliance right and every `cargo run` take
the whole screen. What actually distinguishes them is *who put the machine there*, which is a fact
only a setup program has.

**Two keys are a whole settings file.** Every settings struct is `#[serde(default)]`, so the machine
fills the rest in from its own defaults and writes it back complete on the first start. No installer
needs to know what else is in that file, or track it as the file grows — which is the property that
makes this cheap enough to be worth doing in three places.

**Only where there is none, and that is the whole of the safety argument.** `firstrun.rs` refuses to
put its SoundFont request inside `settings.json` because an installer would have to *overwrite a
settings file it did not create — losing whatever an upgrade was standing on — or parse and merge
JSON it does not own*. Writing only when the file is absent is neither: nothing is parsed, nothing is
merged, and the write happens only where the setup program is creating the install. An upgrade over a
machine somebody has been using changes nothing. Each carrier spells the same guard its own way —
Inno Setup's `onlyifdoesntexist` (with `uninsneveruninstall`, so the uninstaller's promise that
settings were left alone stays true), a `[ ! -f ]` in the macOS postinstall, a `[ ! -f ]` in the
Debian `postinst`.

**It rides with the machine's own component, never the SoundFont's.** On macOS that means
`scripts/machine/postinstall` rather than `scripts/soundfont/`: the bank is a tick box somebody
chose, and this is what kind of install this is. Unticking a download must not leave a television in
a window.

**Two limits.** A *portable* folder gets nothing: it is not an install, has no setup program, and
cannot know whether it is the machine under a television or a copy somebody is trying out — so both
portable READMEs say it starts in a window and name `--fullscreen` and the settings key. And on
Debian the file reaches the `karaoke` service account's config directory, which is the appliance
case, and not the home directory of a desktop user who installs the `.deb` and runs the menu entry:
`postinst` cannot know who that is, and writing into arbitrary home directories is not something a
package should do. That user is in the same position the systemd unit already leaves them in —
`postinst` deliberately does not enable it either.

**One test is the guard.** An installer's file names no `settings_version`, and a file with none
reads as the current version — it is a new file, not an old one — so it takes the defaults for
everything it does not say. `the_file_a_setup_program_writes_changes_only_fullscreen` asserts the
result is the defaults with one field flipped, so the day a load starts doing something to a fresh
file it fails in the test suite rather than on somebody's television.

## A macOS setup program

**One `.pkg` for all seven products, built with `pkgbuild` and `productbuild`, installing into
`/Applications` and `/usr/local` for every account on the Mac.** The sibling of
`A Windows setup program`, and the argument for having one at all is that row's: every other macOS
carrier is something you unpack, so what a recipient gets is 150 MB they must not move files out of,
no `/Applications` entry unless they dragged one, and six command-line tools with nowhere to be
typed from.

Six component ticks over seven component packages — one of them holds no payload at all, and the
hidden one holds the READMEs and the uninstaller, mirroring the `.iss`'s unconditional README lines —
because `pkgbuild` takes one install location per package and `/Applications` and
`/usr/local/karaokemachine` cannot share one.

**The system domain, and that is the *opposite* answer to Windows' from the *same* argument.** There,
everything the installer configures beyond the files is per-user — the `.kmbuild` association is
`HKCU\Software\Classes` and the PATH entry `HKCU\Environment` — so a machine-wide install would place
files every account can see and configure them for exactly one. Here the identical reasoning lands
the other way: the association is declared by a bundle sitting in `/Applications` and the "PATH entry"
is a symlink in `/usr/local/bin`, and both of those are machine-wide by convention. One administrator
prompt is what that costs, and on macOS it is what every installer costs; a home-domain install would
put applications in `~/Applications` and tools under `~/usr/local`, which is not a shape anybody
expects.

**A `.pkg` rather than a `.dmg`**: a `.dmg` is a folder you drag from,
which is the carrier `dist/bin/macos` and the zips already are, and it has no components, no install
step and nowhere to put a command so it can be typed.

**What it deliberately does not do**: no auto-update, no Sparkle, no Homebrew cask.

**One thing it gets for free that the zips do not**: the applications it places are written out of a
payload rather than downloaded, so they carry no quarantine flag and open normally — the `xattr` line
the staged bundles need does not apply to an installed one.

**Removal is a double-click.** `uninstall.sh` removes the bundles, the `/usr/local/bin` entries, the
folder and the receipts, matching entries by what they point at so it cannot eat Homebrew's — but it
lives at a path you can only reach by typing it. Measured against the other three carriers macOS is
the outlier and not the script: Windows has an Add/Remove Programs entry *and* a Start Menu icon,
the `.deb` has the package manager, and the tarball has `./install.sh --uninstall` sitting in the
folder somebody already opened. **`Uninstall KaraokeMachine.command` is the tarball's answer
ported**, and the extension is the whole mechanism — the Finder has no handler for a `.sh` and hands
a `.command` to Terminal. It shows the dry run first, asks, and only then asks for a password,
because being asked for an administrator password by a window that has not said what it wants it for
is a thing people are right to refuse; it defaults to no, and it stops rather than treating an
unanswered prompt as consent. It removes nothing itself. **Not an `Uninstall.app` in
`/Applications`**: it would be more discoverable, and it is a fourth bundle, a fourth plist and an
icon decision for a utility that is not a fourth product.

## A setup program for the remote alone

**The offline remote gets a second carrier on Windows and on macOS, holding the windowed program and
nothing else, beside the all-in-one and independent of it.**
`km-remote-setup-<version>-windows-x86_64.exe` and `km-remote-setup-<version>-macos-<arch>.pkg`. The
Windows one is about 5 MiB against the all-in-one's 77 MiB.

**It is the one product a component tick cannot serve, because the person installing it has no
machine.** `A Windows setup program` argues for components on the grounds that the products are used
together and the shared payload is most of the bytes, and that argument holds for every pair of them
except this one: a remote is what somebody holds while *somebody else's* machine plays, so a computer
that wants it wants none of the rest. Offering that person a 77 MiB download with five tick boxes and
a page of instrument-bank terms is not the same offer as a 5 MiB one with nothing to decide.

**The windowed program, and no second thing.** No PATH entry, no console twin, no command in
`/usr/local/bin`, no file association, no instrument-bank tick box, no pre-written settings file.
Every one of those configures something this carrier does not install, and the settings file and the
bank belong to the machine. What travels anyway is the two licence texts and a README written for an
installed build — MIT asks that the notice be in every copy, so that is an obligation rather than
documentation, and the README is the only thing that says how to take this off again.

**Which is why the macOS package still makes `/usr/local/km-remote`, and it is not a second
product.** A `.pkg` has no Add or Remove Programs to register with and no folder somebody unpacked,
so the papers and the uninstaller have to be files it places, and `/Applications` is not where loose
files belong. Windows needs no equivalent: the install folder is already there and Inno writes its
own uninstaller into it. The rule the two share is that nothing a person *runs* is added, and a
`km-remote` on the `PATH` is exactly what neither installs.

**Independent, and asserted at build time rather than intended.** Its own Inno `AppId` and its own
`AppName`, which the install folder and the Start Menu group both follow; its own product identifiers
and receipts on macOS. Inno decides upgrade-versus-second-copy by `AppId` and finds an uninstaller by
it, so a shared one would mean installing the remote took the karaoke machine away — which is why the
Windows driver compares both values against the all-in-one's script and refuses to build when either
matches. One computer may hold both carriers, and removing either leaves the other.

**The exception is one file, and only on macOS.** Both packages place `/Applications/KM Remote.app`,
because `CFBundleIdentifier` names the product rather than the carrier and two bundles may not share
one. The receipts stay separate, so each package still knows what it archived; what the two
uninstallers cannot do is avoid reaching the same application, so each asks whether the other's
receipt is present before taking it. Windows has no equivalent, the two installs being two folders
holding two copies.

**The payload is the remote's own staged folder**, `dist/km-remote/<platform>/`, rather than
`dist/bin/<platform>`. The all-in-one gathers every product because it installs every product; a
carrier holding one of them has no reason to stage the other six, and the difference is two seconds
against six minutes.

**No Linux one, for the reason there is no Linux setup program.** The `.deb` and the tarball already
carry the remote, and a third answer to a question the package manager has answered is not a gap.

## An uninstaller finds its own work, and never by name

**Everything the macOS uninstaller removes, it identifies by something a rename cannot change: an
application by its `CFBundleIdentifier`, a command by what it points at, a receipt by its id prefix.
No list of literal names decides what goes.**

The rule already existed for `/usr/local/bin` and for the reason that directory demands it — on an
Intel Mac it is Homebrew's, so removing a name because it sounds like ours is exactly the damage an
uninstaller must not do. What is decided here is that the *candidate set* is derived too, and that
the same test applies in `/Applications` and to the receipts.

**A list of names can only describe the version it shipped in, and an uninstaller ships after the
thing it has to clean up.** That is the asymmetry: a release renames a bundle, the next uninstaller
is written naming the new one, and the old bundle becomes unreachable by every version that will
ever exist. It is not hypothetical: two installs either side of a bundle rename left three orphaned
bundles in `/Applications`, two dangling symlinks in `/usr/local/bin` and a receipt for a component
that no longer exists — and a full uninstall followed by a full reinstall would have left every one
of them exactly where it was.

**`CFBundleIdentifier` is the key in `/Applications` because macOS already treats it as the
application's identity.** It survives a rename precisely because changing it would make the Finder,
Launch Services and every receipt treat the result as a different program — so a product that
renames its bundle keeps it, and one that changes it has genuinely become something else.
`com.karaokemachine.` is the bundle namespace, and the video downloader that installs beside this
product from its own repository is outside it and is left alone by the same test that finds ours.

**The receipts cannot do this job, and it is the obvious first idea.** `pkgutil` remembers only the
payload of the install that wrote it, so a rename that keeps its component id updates the receipt to
the new name and leaves no trace of the old one. It sees a rename only when the id changed as well —
which is why receipts are swept by *prefix* for their own sake, and are not what decides which
applications go. That prefix runs to two segments, `com.rrgmc.karaokemachine.` and
`com.rrgmc.km-remote.`: `com.rrgmc.` is the owner's and the downloader's receipts sit under it too,
so the segment after it is what makes the sweep this product's own.

**The current names are asserted too.** They are what the round trip checks, and they are the one
thing the script can state without asking the system a question that a hand-forgotten receipt or a
restored Launch Services database could answer wrongly.

**This is `tools/dist/clean.sh`'s problem solved where that script said it could not be.** Its own
comment records that telling a fossil from a bundle needs something better than a list of dead
names, and that in `dist/` the answer is to take one at the moment the surviving name is staged.
In `/Applications` nothing stages, so there is no such moment — the identifier inside the bundle is
what stands in for it.

## Where a command lives on macOS

**`/usr/local/karaokemachine` for the files, `/usr/local/bin` for the names, and the machine gets a
shim where everything else gets a symlink.** `/usr/local/bin` is the first line of `/etc/paths` on
every Mac, so it is the platform's answer to the Windows installer's *"add the installation folder to
my PATH"* task — which is why there is no such tick here, rather than one that edits somebody's
`~/.zshrc`. Not `/opt/homebrew`, which is not ours to write into.

**The directory is created by a script and never archived in the package**, and that is a real hazard
avoided rather than a style: a payload naming `/usr/local/bin` puts its mode and owner in the bill of
materials, and Installer applies those to a directory that already exists — on an Intel Mac that
directory is Homebrew's and belongs to the user, so ours would quietly break `brew`.

**The shim is the part that was measured rather than reasoned.** dyld resolves `@executable_path`
against the *realpath* of the executable, so a symlink in `/usr/local/bin` finds the `lib/` beside the
real file and all six commands work as symlinks. Rust's `std::env::current_exe()` on Apple platforms
does **not** — it is `_NSGetExecutablePath` with no `realpath` — so through a symlink the machine's
`discover_asset_dir` sees `/usr/local/bin`, matches neither the sibling-`assets` branch nor the
`Contents/MacOS` branch, and falls back to `$PWD/assets`: the machine comes up on a sine test tone
over a plain gradient with nothing on screen saying why. Verified both ways on hardware. `exec` from
a two-line shim makes the kernel record the real path and the bundle's `Resources/assets` is found,
so the machine — the only one of the seven that reads anything relative to its own executable — is
the only one that gets one. The installer asserts **both** halves on every build, because an
assertion that only checked the shim would keep passing on the day somebody simplifies
`discover_asset_dir` and makes the shim unnecessary.

## Signing a macOS release

**`KM_SIGN_IDENTITY` unset means ad-hoc; set to a Developer ID it signs everything, and `--notarize`
finishes the job.**

**Ad-hoc is the default, and that is the whole design.** A fresh clone, another developer's Mac and
the CI runners have no certificates; making signing mandatory would make macOS the one platform you
cannot build without an Apple account, and making it automatic on whatever the keychain holds would
mean two machines producing different artifacts from the same command. So it is one environment
variable, absent means ad-hoc, and every report says which build it just made. An unsigned build
reports *"cannot be opened because it is from an unidentified developer"* and needs a right-click →
Open.

**A report line is not enough on its own.** A report is read once, by whoever ran the build; the
file outlives it and gets handed on. So the macOS setup program carries its state in its **name**,
and the three builds cannot overwrite each other — the second half of the `-no-video` rule in
`Video in a release build`, on a second axis.

**Two variables, because they are two certificates**: Developer ID *Application* signs bundles and
the Mach-Os inside them, Developer ID *Installer* signs the product archive, and they are separate
certificate types. The half-signed combination is **refused** rather than produced, because Gatekeeper
judges the archive: signed bundles inside an unsigned `.pkg` change nothing a recipient sees and look
like they worked.

**Notarization is a separate flag and never implied by signing** — it goes to Apple, takes minutes and
needs the network, and a build loop should not pay that; one submission covers every bundle
inside the archive and only the archive is stapled, so the ticket travels with the file and validates
on a Mac that is offline.

**Signing alone buys little**, and the platform says so rather than this being an argument: a
correctly signed, timestamped archive with a valid chain is still `rejected` by `spctl`,
`source=Unnotarized Developer ID`, and the same archive notarized and stapled comes back `accepted`.
So the honest states are three.

**Only one of the three is worth handing anybody, so only that one has a name**: `task
dist:setup:notarized`, beside the `task dist:setup` that produces the ad-hoc build. It is called
*notarized* rather than *signed* because of the paragraph above — signed-only is a real state, it is
the one `spctl` rejects, and a task named for it would promise the wrong half. Sign-only keeps no
task of its own for the same reason; it is still reachable by setting the two identities and leaving
`--notarize` off.

**All three name their file, and the marker goes on the declined build** — so the notarized package
keeps the plain `karaokemachine-setup-<version>-macos-<arch>.pkg`, signed-only becomes `…-unnotarized.pkg`
and ad-hoc `…-unsigned.pkg`. That is the `-no-video` rule reached by the same argument and *not* by
the same mechanical reading: there the plain command produces the plain name, and here the plain
command produces the *lesser* build, so the two clauses of that rule pull apart. What settles it is
which half of the rule is load-bearing — the file anybody is handed should be the one with nothing to
apologise for, and a build that declined something has to say so in the one place that travels with
it. **Three markers rather than two**, because the middle state is the trap: signed-only looks
finished, opens a certificate chain in `pkgutil`, and is still refused.

**They coexist rather than overwriting.** Sharing one name means an ad-hoc rebuild for a local test
silently replaces a notarized package that cost an Apple round trip, leaving the folder with no way
to say which build is in it — the same failure `-no-video` was given a marker to stop. `clean:old`
needed no change: it reads the version as the field after the app name and still finds it in a
marked name. The cost is up to three packages per version in `dist/`, about 94 MB each.

**The three values it needs are committed, in `tools/platform/macos/installer.sh`, and the reason is
that none of them is a secret rather than that they are cheap to replace.** The two identity strings
are certificate common names, which `pkgutil --check-signature` prints from any package signed with
them; `KM_NOTARY_PROFILE` is the label of a keychain profile whose Apple ID and app-specific password
never leave the data-protection keychain. What they *do* carry is a person's name, and that is
answered under *Published identity* in `What a committed file may say about the machine it was
written on` rather than here. An environment variable still wins over each, so signing as somebody
else needs no edit to a tracked file.

**This narrows "ad-hoc is the default" to every path except `--notarize`, and not one step further.**
The defaults are applied only when that flag is given, because `dist_signing` keys off
`KM_SIGN_IDENTITY` being non-empty and a default applied unconditionally would make every build here
sign — costing exactly the property the paragraph above is about, that a machine with no certificates
can still stage a release.

**The Read Me pane explains two of those three and says nothing about the third.** Unsigned and
signed-only both put an obstacle in front of the reader — a double-click that reports the file
cannot be opened — so each gets a snippet saying what to do about it. A notarized build substitutes
nothing: it opens like anything else somebody installs, and a heading announcing that it is signed
and notarized tells them only that what is about to happen is what they already expected. **A pane
of reassurance is the pane people stop reading**, and the paragraph that matters — how to remove
this later — is what then goes unread with it.

**The same reasoning keeps the administrator password out of it.** Every installer writing to
`/Applications` asks for one and Installer's own UI is what asks, so a heading about it describes
the weather. The uninstaller is the opposite case and still says so: that is a thing somebody runs
by hand and can be surprised by.

**Windows stays unsigned** — that needs a different certificate nobody has, so the two platforms'
signing stories are separate.

## What a macOS bundle says it is for

**`LSApplicationCategoryType` on all four bundles: Music for the machine and the remote, Utilities for
the package builder and the assets tool.** It is one string per manifest, and macOS shows it in Get
Info, in Launchpad and on an App Store page — a bundle without it leaves blank the line every other
application fills in.

**Music rather than Entertainment, and that is the only one of the four that was argued.**
Entertainment is the shelf the system keeps for games and film players. What a person does with this
is sing a song, and that a *video* song happens to be a video does not make the machine a video
player any more than the MP3+G songs make it a picture viewer. The remote takes Music because it is
the same product's other half: filing a remote apart from the thing it controls would read as two
unrelated programs wherever the system lists both.

**Utilities for the two that make and fetch content, because neither plays anything.** The package
builder reads a folder of somebody's songs and writes a package; the assets tool fetches soundfonts
and wallpapers. Both are tools about files, which is what that shelf is for — and calling either of
them Music would have been the convenient answer rather than the true one.

**It is advisory, and that is why it needs a check.** Nothing at run time reads it: not Gatekeeper,
not a permission, not a capability. So a manifest declaring none — or naming a category macOS does
not recognize, which it ignores without a word — produces a build that succeeds, installs, runs, and
is filed nowhere, with no line in any log to say so. `tools/platform/macos/installer.sh` reads every
`tools/platform/macos/Info*.plist` by glob and refuses a missing or unknown value. **Found by glob
rather than named**, so a fifth product is covered the day its manifest is added rather than the day
somebody remembers the list — and the same pass checks `LSMinimumSystemVersion` across all four.

## What the machine *is*, on iOS

**An application on an iPhone and an iPad, and the same program again rather than a fourth one.**
`crates/machine/km-machine-ios` is a `staticlib` over the machine's own library, `ports/machine/ios/`
is the XcodeGen project, and the synthesizer, the catalog, the display and the API are untouched. A
*requirements* row rather than a technical one, because it makes a phone and a tablet supported
places to run this product rather than only to hold its remote.

**A `staticlib` where Android has a `cdylib`**, for the reason
[`The offline remote as an iOS application`](remotes.md#the-offline-remote-as-an-ios-application)
gives: iOS will not load an arbitrary dynamic library and forbids `fork` and `exec`, so linking the
machine into the app binary is the only shape available. `SDL_main` is the entry point on both
platforms, so what differs is who calls it: `SDLActivity` there, `SDL_RunApp` here.

**The two directories are created in Swift and handed down, never derived in Rust**, which is the
seam [`Where an iPhone keeps its favorites`](remotes.md#where-an-iphone-keeps-its-favorites) already
argues for the remote. The data directory is `Library/Application Support/karaokemachine`, and
packages live in `Documents/packages`, which is what `UIFileSharingEnabled` puts in the Files app and
in Finder over USB. `directories` ships no iOS module and would answer with a macOS path outside the
container.

**Assets are ordinary files, so nothing unpacks them.** Android carries an unpacking step because an
APK's assets are not files and only the asset API can read them. A bundle is a filesystem, so the
tree goes in as a folder reference and the machine finds it beside its executable.

**A font ships in the bundle**, the treatment the Linux tarball gets, because this platform's system
font paths are undocumented and a path taken from documentation rather than off a machine is the
mistake [`A CJK face is borrowed, never bundled, and opened only when asked for`](#a-cjk-face-is-borrowed-never-bundled-and-opened-only-when-asked-for)
records. **CJK stays a gap**, reachable through `display.font_cjk`.

**The audio session is configured before SDL starts**: category `.playback`, then active. cpal's
CoreAudio backend does not do this, and without it the silent switch mutes the machine and a song
stops the moment the application leaves the foreground. It fails quietly, on a platform where the
obvious reading is a broken audio path rather than an unasked-for session.

**ffmpeg travels as four embedded frameworks rather than a static link, and the reason is the
license.** Shipping the shared libraries beside the binary is compliant under LGPL where linking them
into it would not be. iOS permits dynamic libraries inside a bundle and refuses only ones loaded from
outside it, so the posture the APK already has survives whole. The license text travels with them.

**The machine advertises nothing here.** Apple has required
`com.apple.developer.networking.multicast` since iOS 14 and grants it only after a manually reviewed
request; without it an advertisement is dropped and nothing says so, which is the failure shape
[`Finding a machine without multicast`](remotes.md#finding-a-machine-without-multicast) refuses on
the remote's side. The iOS remote finds a machine by unicast sweep against `GET /api/v1/discover`, so
an iOS machine is discoverable by it with no entitlement anywhere, and the Android remote is given
the address by hand. The advertiser is a seam, so an entitlement or an `NWListener` in Swift is a
later answer rather than a redesign.

**A machine in the background stops answering its API, and that much is accepted rather than fixed.**
iOS suspends an application that claims no background mode, and this one claims none: it pauses when
the screen goes away, so keeping it awake would buy a remote a machine nobody can see, at three
points of battery an hour. See [`The machine sleeps when it leaves the screen`](audio.md#the-machine-sleeps-when-it-leaves-the-screen).
Android answers the rest by being an appliance under a television, and a tablet is not one.

**A machine that is back on the screen and still not answering is a different thing, and it is not
accepted.** Being suspended costs the API while the application is away; it must not cost it
afterwards, because a machine somebody is looking at, that draws an address nothing answers, is
unreachable with no sign of it anywhere. The system destroys the listening socket during a suspend,
so returning to the screen is when the machine takes its port again. See
[`The machine holds its port, rather than claiming it once`](api-and-network.md#the-machine-holds-its-port-rather-than-claiming-it-once),
which is platform-neutral: an appliance loses a socket with an interface rather than to a suspend,
and the answer is the same one.

**Four of the six questions the interface asks a platform are false here.** There is no desktop to
leave fullscreen for, no drop event, nothing that reveals a folder in the Files app, and no browser
to open, so the controls that would need them are compiled out rather than left to fail. The number
pad defaults on, for Android's reason: it is the only way to enter a song number where there is no
keyboard.

## An iOS carrier is unsigned, and the person installing signs it

**Both iOS applications travel as an `.ipa` with `unsigned` in its name**, and whoever installs one
signs it with their own Apple ID. `karaokemachine-<version>-ios-unsigned.ipa` is the machine and
`km-remote-<version>-ios-unsigned.ipa` is the offline remote, each a zip holding `Payload/<name>.app`
and nothing else, written by `dist_ipa` from the bundle `tools/port/*/ios/build.sh` already compiles.

**No signature this repository can make would help.** Apple defines four ways off a page like this,
and each asks for something a release cannot supply:

| Route | What it costs |
|---|---|
| App Store, TestFlight | App Store Connect, a review, and a build that stops working after 90 days |
| Ad Hoc | every recipient's device identifier, collected in advance, 100 a year |
| Enterprise | a separate programme, and its terms allow employees only |
| Web distribution | the European Union only, against criteria Apple approves per developer |

A development certificate — the one `project.yml` names, for putting a build on your own devices —
produces a file that installs for its author and fails for everybody else. So the choice is an
unsigned carrier or none, and none means the answer to *how do I run this on my phone* is a Mac, the
full Xcode and a checkout.

**`--ipa` refuses a debug build, and the machine's refuses `--no-video` as well**, for the reason
the macOS installer has no `--no-video`: a person who downloads one file is not choosing a feature
matrix, and neither a debug build nor a video-less one says what it is in the file name.

**How to sign and install one belongs in [`README.md`](../../README.md#installing)**, not here. It is
the one carrier whose instructions are a procedure the recipient carries out rather than a
double-click, and the README is the document written for somebody who has the machine rather than
the source.

## A log file for the runs nobody is watching

**`--log-file`, or `KM_LOG_FILE=1`, puts the log in a `logs` folder in the application's own data
directory — one file per run, the ten newest kept.** This is `What a shipped build says out loud`
reaching the case that row does not cover: it gets the *volume* right and leaves the *destination*
alone, and the destination is where the machine, the package builder and the offline remote all lose
their log entirely.

All three ship a GUI-subsystem executable on Windows so that a double-click opens a window and no
console beside it — and a process with no console has a null standard output handle, so `tracing`'s
writer does not fail, it **discards**. Every line, silently, for the whole run. `km-console` stops
that being a *panic*; nothing stops it being a loss, so "it did not start and I do not know why" has
no answer short of finding a terminal and running the thing again — which is exactly the state in
which a person cannot.

**Four choices inside it.** It is *asked for by name*, on the same argument `--frame-stats` is: a
log level says how much detail you want, and whether the program writes a file is not that question,
so it does not ride the `-v` ladder and the ladder still decides what goes in it. It is *both*, not
instead: the console prints exactly what it printed before, because a terminal run that went silent
on being asked for a file would surprise anybody. The file is *never colored and always timestamped*
even where the console is neither — ANSI is for a terminal, and the machine drops its own clock
under systemd because journald supplies one, which is right for the journal and wrong for a file,
where nothing else will. And it is *per run, ten kept*, because the unit anybody asks about is a run
— "the log from when it broke" is one file here and a guess under a daily rotation — with the count
bounded so the folder stays readable.

**There is a settings key, `logging.file`, and the machine that needs it is the one nobody can pass
an argument to.** A flag reaches the run somebody types and an environment variable reaches the unit
somebody wrote; neither reaches a box under a television that a person walks up to and starts by
double-clicking its icon — which is at once the machine most likely to fail with nobody watching and
the one hardest to make fail again on purpose. A machine being worked on is a standing state, not an
evening.

**Three programs read that key, and they read one section.** The machine, the package builder and
`km-admin` each keep a settings file and each can be started from an icon, so the argument above
reaches all three alike; the offline remote keeps none, and inventing one to hold four keys would be
the tail wagging the dog. What stops three readers becoming three dialects is that the section is
one crate — `km-logsettings` — which the `A crate rather than three copies` note in
[`km-logfile`](../../crates/platform/km-logfile/src/lib.rs) sets the test for: the `-v` ladder stays
duplicated because its rungs name a different crate in every program, and a grammar does not, because
three copies of one would be three answers to what a key accepts.

**Narrowest source wins**: the flag, then the variable, then the file. That is the order every other
setting here uses, and it is what keeps one run's `--log-file` from being an edit to the machine.

**Not under `debug`**, which is the one place it looks like it belongs. `debug.enabled` publishes a
passwordless copy of the whole API, and asking a machine to write down what it did must not be a way
to arrive at that.

It is read by a **second, narrower pass over the file** rather than by `Settings::load`, because the
subscriber has to exist before anything can be said and `load` writes — so asking where the log goes
through it would create a `settings.json` for a machine that has never run. A file that will not
parse peeks as no opinion at all and is reported a moment later by `load`, with the rename to
`settings.json.bad` that goes with it.

The name is UTC and says so with a `Z`, since a local-time stamp needs a time zone database and this
is written to need nothing at all. The three mobile shells are **out of scope and not by oversight**:
Android has logcat and iOS has Xcode's console, and both are real destinations rather than
`/dev/null`.

**A third destination takes the same stream and answers a question a file cannot**, and it is where
the *asked for by name* half of this stops applying. [`The machine's own log is a route, and it is the
owner's`](api-and-network.md#the-machines-own-log-is-a-route-and-it-is-the-owners) keeps the most
recent records in memory for the API to serve, unconditionally — a file costs a directory that fills
up, so somebody decides, where a bounded ring costs a fixed amount and nobody has to. The ladder
governs all three alike.

## A log that goes to a viewer instead of a console

**`--ecapplog` sends this run's log to the
[ECAppLog](https://github.com/RangelReale/ecapplog) viewer, and builds no console layer while it
does.** The machine, the package builder, the offline remote and the picture-and-bank tool all carry
it, and it takes an address — `--ecapplog=192.168.1.x:13991` — to reach a viewer on another computer,
loopback being where one listens by default.

**This is the destination for the run somebody is watching**, where the three beside it all answer
after the fact. A console is a scrollback and a grep; a file has to be found before it can be read;
the ring behind `/admin/logs` holds the last few hundred records for a person who has thought to go
and look. A window beside the run with a tab per crate, a level to colour by and a details pane is a
different thing from all three, and it is what somebody watching a scan or a start is actually after.

**A tab is a crate, and a dependency that speaks the `log` facade gets one too.** A `tracing` target
is the module path a line came from, so filing by it outright opens several dozen tabs at a
granularity nobody reads at; the crate is the unit somebody asks about. The facade is the half of
that rule which takes work: `tracing-log`'s bridge carries a `log` record under a target of its own
and puts the record's real one in a field, so a layer reading the target off the metadata files
`mdns_sd` and every other such dependency together under one tab called `log`, none of them with a
call site. The viewer's layer recovers the record's own target, and the tab is the crate that target
names. **The target and not the module path beside it**, because the target is what `RUST_LOG`
selects on — a tab a filter cannot name is a tab nobody can turn off.

**It replaces the console rather than joining it**, which no other destination here does. Both at
once print every line twice, and between a terminal and a window offering a filter there is no
question which one a person reading is reading. The *file* and the *ring* are untouched, because
where the detail goes and how much of it there is are different questions — `--ecapplog --log-file`
is a run whose log is in two places and whose terminal is quiet.

**The verbosity ladder still governs it**, through the same `EnvFilter` the three beside it sit
under. That is the half [`What a shipped build says out loud`](#what-a-shipped-build-says-out-loud)
settles outright: a level says how much detail, and where the detail goes is not that question. `-v`
and `RUST_LOG` reach the viewer unchanged.

**The same three rungs the file has, narrowest first**: `--ecapplog`, then `KM_ECAPPLOG`, then
`logging.ecapplog` in the machine's `settings.json`. A run says what this run does, and neither a
variable nor a settings file may override it — the order
[`A log file for the runs nobody is watching`](#a-log-file-for-the-runs-nobody-is-watching) sets and
every other setting here keeps. Two destinations with one ladder is one rule to remember rather than
two.

**The variable is the rung that matters most here**, which is the reverse of the file's case. That
one has `KM_LOG_FILE` for a systemd unit; this one has `KM_ECAPPLOG` because somebody working on this
wants it on for *every program at once*, and `.cargo/config.toml`'s `[env]` block is where a checkout
already says such a thing — `KM_NO_MDNS` is there for a neighbouring reason. Nothing an owner
installs reads it, a release running from a package never going through cargo. `1` is the viewer on
this machine and an address is one anywhere else; `0` is the spelling that says no, which a
checkout-wide entry makes necessary, and which a single command can still put in front of one run.

**The settings key reaches the three programs that keep a settings file**, and it is the same
`logging` section in all three — the machine's, the package builder's and `km-admin`'s. What the key
adds over the variable is a program that is not started from a shell: the box under a television,
being worked on, which is a standing state rather than an evening, and which is the whole argument
`logging.file` already won. The two tools reach it by the same road, an icon rather than a command
line. `true` is the viewer on that machine, an address is one anywhere else, and `false` is a no
somebody can write down without deleting the line.

**The offline remote is the one left out**, because it keeps no settings file and a file invented to
hold this section would be a file nobody would find. Its flag and its variable are the whole of what
it reads.

**A run-level section in a curator's file is the one thing here that had to be argued.** The package
builder's settings are otherwise a vocabulary that follows a person from one corpus to the next,
where this follows the box. They share a file because the alternative is a second one in the same
folder holding four keys, which is worse on the only axis that decides it: somebody looking for where
to turn the log on has one place to look.

**A value nobody can read costs the viewer, not the start.** A settings key that will not parse
answers as no opinion and is reported by `warn_about_unread_settings`, exactly as a bad `logging.keep`
is; a variable that will not parse is reported the moment there is a console to report it on. Neither
is guessed at as the default address: a typo that quietly became `127.0.0.1:13991` is a viewer
somebody would go on looking for on the machine they meant to reach.

**The four programs that build a subscriber, and not the other four.** `km-pack`, `km-lyrics` and
`km-carols` emit no `tracing` events at all — they speak through `km-console`, having one thing each
to say and a terminal to say it in — so giving them a viewer would be giving them a subscriber first,
to carry nothing. The mobile shells are out for the reason the file decision gives, and for a second:
there is no command line on a television to type this on, and
[`What a shipped build says out loud`](#what-a-shipped-build-says-out-loud) records that the
environment is not a route in there either.

**An address needs a port and that is the whole of the check.** The protocol has no default port a
client could fill in, so a bare host is a viewer that is never reached; past that, the connection
retries and can answer a bad hostname better than a parser guessing at one.

**The value takes an `=`, and that is not style.** The machine takes a double-clicked package and the
package builder takes a folder, and an optional-value flag written with a space swallows whatever
follows it — so `karaokemachine --ecapplog song.kmpkg` would connect to a viewer called `song.kmpkg`
and open nothing. The space form is refused in all four, including the two with no positional
argument today, because a spelling that depends on whether one has been added yet breaks when one is.

**The address is said by the banner**, and by `--show-paths` on the machine, rather than where the
connection opens. A subscriber is built before a program has settled whether it has a console, and on
Windows the answer *no* makes a `println!` a panic rather than a discarded line — which is the fault
`km-console` exists to prevent, arriving by the one door it does not guard.

**The viewer does not have to be there.** Lines queue while it is unreachable and arrive when it
opens, so a program can be started first and attached to afterwards, and the viewer can be restarted
mid-run. Nothing reports the absence: it is the ordinary state this is built to sit in, and a hook
saying so would fire on every reconnect through a run behaving exactly as intended — and could not
say it through `tracing` anyway, an error reported by the log's own destination being taken by that
destination and dropped.

**The queue is drained on the way out**, because a layer handed to a global subscriber is never
dropped and a queue that dies with the process loses the last entries, which are the ones somebody
watching is there for. One second is the ceiling, and only the failure ever reaches it: draining to a
connected viewer is a loopback write that finishes in microseconds, and a program somebody has
finished with must not pause on a window nobody is looking at.

## A panic writes a file even when nothing else does

**A panic in the machine writes `<stem>-<UTC>.crash` into the same `logs` folder, on every run,
asked for or not.** It holds where the panic was, what it said, which thread it was on, the build's
version and a backtrace. The same facts also go out as a `tracing` event, so a run with a console
prints them.

**The machine, because the machine is the one nobody is sitting in front of.** The hook is in
`km-logfile` beside the file, so the package builder, the offline remote and `km-admin` are a line
each away from it and should take it when a silent death costs one of them an afternoon. What made
this urgent is the box under a television: a program with a window somebody is looking at least
*vanishes* visibly, and a machine that is playing a song to a room is one whose owner has no console
to go back to and no way to make it happen again.

**This is the case [`A log file for the runs nobody is watching`](#a-log-file-for-the-runs-nobody-is-watching)
does not reach, and the gap is not one of volume.** A panic does not travel through `tracing` at
all: the default hook writes to stderr, and a GUI-subsystem executable's stderr is the same null
handle its stdout is. So the machine can die of a panic and leave *nothing* — no message, no dump,
no marker — and a run that had `--log-file` on the whole time has a log that stops mid-sentence at
the last ordinary event, which is the most misleading of the three outcomes because it looks
complete.

**Unasked-for, and that does not reopen the argument that made the log a flag.** What was refused
there was a machine writing files for a year because of one bad night. Nothing is written here until
a panic happens, so a machine that does not panic never writes a byte — and a person cannot decide
in advance to record the one event they will want, because by the time they know they wanted it the
process is gone.

**A second extension rather than a second folder.** Retention counts by extension, so runs and
crashes retire on their own clocks in one directory: an evening of starting and stopping cannot push
out the report of the panic that ended one of those runs, which is the one file in that folder
anybody is looking for.

**Best effort throughout, and it has to be.** A hook that can fail is a second panic inside the
first, which Rust answers by aborting — taking with it the report this exists to write. So nothing
in it can panic, the file is flushed rather than left to a drop that may not run, and a directory
that cannot be made costs the report rather than the process.

### How many are kept is a setting, and `all` is one of the answers

**`--log-keep <count|all>`, or `KM_LOG_KEEP`, replaces the ten.** Naming a count turns the log file
on by itself, because asking to keep a history is asking for one to be written and a flag you have
to remember to pair is a flag you will forget to pair.

**`all` is what a machine being worked on wants.** The names already carry the date and time the run
started, so a folder that is never pruned is that machine's whole history in the order it happened —
which is the thing you need when the interesting run was four days and thirty starts ago, and
exactly what a ten-file window is guaranteed not to have. The default stays ten, because the reason
for it is unchanged: a folder somebody opens is a folder that should be readable.

**One environment variable covers all four programs.** Only the machine grows a flag; the package
builder, the offline remote and `km-admin` read the variable where they open their file, so a box
set up once keeps everything without three more command lines learning an option they would rarely
be given.

**`logging.keep` says the same thing in the settings file** of each of the three that keeps one, and
takes either shape a person would type — `"all"` or `200` — because a configuration file that accepted only one of them would be
answering a question about serialization with an error message about JSON. A word that is not `all`
leaves the usual number standing and **says so in the log**: a value nobody can read is worse than a
wrong one, since the machine goes on keeping ten while the person who set it believes it keeps
everything, and they find that out on the evening they go looking for the run that broke.

## A fourth program, rather than a fourth tab on the owner's page

**`km-admin` — KaraokeMachine Admin — finds pictures and SoundFont banks and sends them to a
machine.** It is a desktop program with a window, staged into every carrier beside the machine, the
package builder and the offline remote.

**The obvious alternative was a tab on `/admin/`, and it is wrong for three reasons that compound.**
That page is already the owner's, already speaks an end user's vocabulary, and already takes files —
so a "find some" tab beside "upload one" reads as the natural place. What rules it out is where that
page *runs*: in the machine's process, on a box under a television.

- **The machine may have no way to the internet at all**, which is one of the two cases this exists
  for. `Nothing downloads` permits fetching a bank on an explicit instruction, and that is as far as
  it goes; a machine that searches stock photograph APIs is a different product from the one that
  decision describes.
- **It would put somebody's Pixabay key on the appliance.** A credential belongs where its owner is
  sitting, not on a shared box in a living room that every phone in the house can reach.
- **A hundred JPEG decodes is not what a machine should be doing while it plays a song.** `analyze`
  is `rayon`-parallel and CPU-bound for minutes; the display loop is already the other thing
  competing for those cores.

**The television box is the case that settles it rather than softens it.** On Android there is no
shell, no file manager that reaches app-private storage, and no way to put a `.sf2` anywhere the
machine looks. Without this, a bank the machine cannot fetch itself is a bank that box cannot have. A
desktop program that downloads and uploads is the only shape that reaches it.

**It is `tools/cmd/` and not `crates/`**, on the division those directories already draw: `crates/`
is what the product *is* and `tools/cmd/` is the commands somebody types. This is the fourth program
in the product and the third that happens to have a window, which is a property of how it is used
rather than of what it is.

### It also sends a file you already have, and that does not weaken any of the three reasons

**Three cards, one per kind, and a Songs tab that finds nothing.** The three arguments above are all
about *searching* — the internet, somebody's credentials, a hundred JPEG decodes — and forwarding a
file touches none of them: nothing is searched, nothing is decoded, and no key is involved. So this
is not the tab on `/admin/` that was ruled out; it is the same program doing the other end of the
same errand.

**Songs are the case that needed it and could never have been searched for.** A package is somebody's
own and no stock library has one, so there was never a "find me some" for this tab to be. What there
is is a gap: `/admin/` takes a `.kmpkg` from a browser that can reach the machine, and a television
box has no shell and no file manager that reaches where the machine looks — so a package on a laptop
has no route onto that box at all unless its owner already had the machine's page open and working.
Pictures and Sound take one too, for the smaller version of the same case: the `.sf2` somebody
bought, the photograph they took.

**What it is not is a second `/admin/`, and that is a claim about the searching rather than about the
machine's contents.** The installed-package table, the wallpaper rotation and the machine's own bank
list are on both surfaces: one page set draws those controls, so a host rendering them is not a
second place to keep right. What it buys is an errand that does not cross programs — somebody sends a
package from here, sees it land in the wrong block of a thousand, and fixes it here.

**A tool deleting across the network is not a wider blast radius than the page somebody is standing
at.** Every write this program makes crosses that network: it renames the machine, changes its
password, chooses its audio output and uploads gigabyte files to it, each with the same admin token a
delete needs. A surface trusted with the password is not one to withhold a delete from.

**Three things are off, each for a reason of its own:**

- **The Problems tab** is unanswerable rather than withheld: its rows are identified by a path
  `PackageProblemDto` refuses to publish, so a host over HTTP has nothing to draw. See
  [`The Problems tab is the machine's own page only`](#two-admin-surfaces-one-vocabulary).
- **The password *reset*** draws a new PIN on a television this program is not beside — the
  changing-versus-resetting distinction the paragraphs below draw.
- **A package's size** is not a control and is missing all the same: `PackageDto` publishes no byte
  count, so that column is empty here and holds a number on the machine's own page.

**A flag alone delivers none of this.** Each control needs a real answer from `Songs`, `Sound` or
`Pictures` — eleven trait methods over HTTP, seven entries in the `Call` table checked against the
machine's own route surface, three DTO conversions, and a percent-encoder for ids. An id in a path
that is not escaped while it is still a separate value lets `RemoveBank("../../admin/password")`
address another route entirely, which `an_id_with_something_awkward_in_it_cannot_reshape_a_url`
holds down.

**Three exceptions, and they are the setting-up ones: the machine's name, demo mode, and the
password it gave itself.** This is the program somebody has open while a machine is being set up —
it is where the machine was *found*, where its address was typed and where its first package,
picture and bank are sent from — and a name is the other thing that gets decided in that same half
hour. Walking to `/admin/` to type one is the seam that made the exercise feel like two programs
rather than one job.

**The password is the third under the same argument, and the strongest case of the three.** A machine
arrives answering to a six-digit PIN drawn on its own television, so *every* machine starts in the
state that wants this act — where a rename is optional and demo mode is a preference. And this
program is the one that already knows: it reads `factory_password` off `/discover` in order to point
at a machine at all, so it can say so without asking anything it was not asking anyway. It nags on
every page and offers the change on the Machine tab, which is exactly what `/admin/` does, for the
reason recorded there.

**It changes the password and does not reset it**, which is where this stops short of `/admin/` on
purpose. `{"password": null}` puts a machine back on a freshly generated PIN, and the only place that
makes sense is beside the screen that will then show it. What is missing from a box under a
television is the *first* change; the recovery is not missing, it is somewhere better.

**The test is not "is this an admin action" but "is this decided in the half hour when a machine is
being set up"**, and it admits more than these three: correcting the block a package landed in is as
much a setting-up act as typing the machine's name, in the same half hour.

**"A second place to keep right" is answered rather than ignored**, by
[`Two admin surfaces, one vocabulary`](#two-admin-surfaces-one-vocabulary): the two say the same words
for the same controls, so a person who has learned one can read the other. And neither holds any
policy of its own — both send `PUT /api/v1/machine/name` and `PUT /api/v1/demo`, which decide what a
name may be and what turning the mode on means; a refusal is passed through in the machine's own
words. What is duplicated is a form, not a rule.

**Both are admin-only routes, and that needs no special case here.** This program's standing shape is
*try, and ask for the password only if refused* — so on a machine with a password these two answer
with the login form the page already has.

**It validates the extension and the size, and nothing else.** Both numbers come from the machine's
own `km_api::uploads` table rather than from constants copied here, so the chooser says what the
machine takes rather than what somebody once wrote down. Everything past that is the machine's
answer, passed through in its own words: it is the authority on what a `.kmpkg` contains and on what
a name may be, and a second opinion here would be a second thing to keep in agreement with it.

## Two admin surfaces, one vocabulary

**`km-admin` and the machine's own `/admin/` are two programs doing one job, so they use one set of
words — the machine's.** Where a page exists on both, it has the same name, in the same case, in the
same place in the list.

### …and one page set behind two hosts

**The compiler holds this rule rather than a person.** What it asks for — the same page under the
same name in the same place — is not something two independent implementations can be kept at by
hand: a five-pane settings strip, a Sound output picker down to `OutputRow::is_system_choice`,
`fell_back`, `changeable` and `?all=1`, the output level with its disclosure link and its question
before a large rise, and a factory-password banner are what drift when they exist twice.

**`km-admin-pages` is a trait-per-tab crate serving both**, which is `km-remote-pages`' arrangement
one layer over for the singer's remote. The machine implements the traits in its own process;
`km-admin` implements them over HTTP. `docs/architecture/admin.md` is how it is built.

**The rule above survives and is not redundant**, because two things it governs are still two things:

- **The words.** Each surface draws from the catalog beside its own markup: the shared pages from
  `km-admin-pages/i18n/`, `km-admin`'s own half — Home, the picture search, the bank table — from
  `km-admin/i18n/`. Both are translated, and neither file holds the other's keys, which is what
  [`Catalogs live beside the words they translate`](repository.md#catalogs-live-beside-the-words-they-translate)
  asks for. **This rule is what keeps the two agreeing**: *Send*, *Remove*, *Size* and *Pictures* are
  in both files, and a control that means the same thing on both surfaces is worded the same in both.
- **What each surface *does*.**
  [`A fourth program, rather than a fourth tab on the owner's page`](#a-fourth-program-rather-than-a-fourth-tab-on-the-owners-page)
  draws that line and still draws it, unchanged in every reason it gives. What changed is only the
  *mechanism*: the searching half is `km-admin`'s own routes merged over the shared router rather than
  a second program's markup, and the machine cannot serve it because nothing there implements it.

**Read that last point carefully before concluding a decision was abandoned.** The three reasons
searching must not run on the machine — it may have no internet, it would put somebody's Pixabay key
on an appliance every phone in the house can reach, and a hundred JPEG decodes is not what a box
should do while playing a song — are untouched. A shared *page set* is not a shared *program*.

### The searching is a page under its tab, one click from the tab itself

**`km-admin`'s Pictures tab was one page with three cards — send one you have, search, review — and
the searching is now a page of its own at `/admin/pictures/find`, reached by a link.** The bank table
moved the same way, to `/admin/sound/fetch`. The tab strip is unchanged; what changed is that the tab
lands on the machine's own Pictures page and the searching is a step past it.

**This is a consequence of sharing rather than a preference, and it is worth being honest about
which.** The shared Pictures page is `km-admin-pages`' markup, and askama gives no way for a host to
add a card to somebody else's template. The alternatives were both worse:

- **Move the searching into the shared crate**, behind capabilities. That means the provider chooser,
  the key box, the review grid and the job machinery in the crate the *machine* links — plus `dyn`
  traits over a progress sink and `km-wallpaper-pack`'s own types, with exactly one possible
  implementation forever. A seam with one implementation buys nothing, and it would put a thousand
  lines of code the machine can never run inside the machine.
- **Keep km-admin's own Pictures page instead of the shared one.** Then the send control, the
  factory-password banner and the tab strip stay duplicated on the very tab this exercise was about.

**What it costs is one click on the two pages that search.** What it buys is that everything those
tabs have in common with the machine's page is one copy. Given the errand — somebody sets a machine up
once, searches for pictures once, and thereafter sends files — a click before a several-minute search
is the cheapest thing on the page.

**The front door is the third page of this program's own**, at `/admin/connect`, and it has no
counterpart on the machine: a machine opens inside the machine it is about and has no say over which
machine that is. See
[`The front door is where a tool is pointed and let in`](#the-front-door-is-where-a-tool-is-pointed-and-let-in).

**Two names for one job let the two drift without either looking wrong from inside itself.** That is
exactly what
[`One spelling per concept, across every surface`](foundations.md#one-spelling-per-concept-across-every-surface)
is about, and the tie-break it gives applies here: **the machine's spelling wins**, because the
machine's pages are the ones a person reaches without installing anything.

So `km-admin`'s first tab reads *This machine*, which is `tab-machine` in
`km-admin-pages/i18n/en.ftl`, and the three that follow — Songs, Pictures, Sound — are already the
machine's own order. `/admin/` puts *This machine* at the front to meet it, which is where it
belongs anyway: the tool put it first because every other page there is useless without an address,
and the same is true of the machine's page for a different reason — it is the tab about the machine,
on the machine's own pages.

**Two pages have no counterpart, and neither is a divergence.** `km-admin`'s **front door** is the
page before the strip, and `/admin/` needs none because it opens inside the machine it is about.
`/admin/`'s **Problems** reads faults out of the machine's own state, and a tool talking to it over
HTTP is not where that belongs.

**Two controls arrived on both surfaces at once, and this rule is what decided where.** The output
device went on **Sound** on both, because it is about sound and Sound is what both surfaces call that
page — not on *This machine*, which would have made it installation configuration on one surface and
sound on the other. The development console's switch and the frame-statistics switch joined debugging
in the **same pane** of *This machine* on both, which is what kept the two tab strips identical: a
pane on one surface and not the other would be exactly the drift this rule exists to catch, and a
person who has learned one surface would find the other's strip a different shape.

**What this does *not* say is that the two surfaces do the same things.**
[`A fourth program, rather than a fourth tab on the owner's page`](#a-fourth-program-rather-than-a-fourth-tab-on-the-owners-page)
draws that line and still draws it. One vocabulary is what makes the line legible: a person who has
learned one surface can read the other and see what is missing, instead of wondering whether it is
merely called something else.

### One stylesheet, and the tool wears the machine's amber

**Both admin surfaces load `km-admin-pages/static/admin.css` and nothing else.** `km-admin` keeps no
stylesheet: the searching's panels, provider cards, review grid, job bar and toasts are the last
section of the shared file, in that file's own variables. A second stylesheet holds a second name for
every colour and every radius, which is what lets one settings strip be built twice.
**So the machine downloads a few kilobytes of CSS it never uses.** [`Bundling assets`](#bundling-assets)
admits an exception only for size on the order of the 31 MiB SoundFont or for a license that forbids
redistribution, and `km-remote-pages` applies that to a much larger case: a quarter-megabyte of jsQR
lands in the machine's binary for a page the online mode never draws, and a cargo feature is refused
there because it *"would make it the first compile-time mode split in a crate whose header says the
templates branch on capabilities and never on a mode."* A few kilobytes of rules for a page one host
does not serve is the same trade, two orders of magnitude smaller. One file, one `ASSET_VERSION`, one
place each colour is spelled.

**Both surfaces wear the machine's amber.** `km-admin` draws the machine's chrome, under the machine's
name, with the machine's tab strip; a second lead colour under that heading reads as two products
rather than one.

**What differs is the *mark*.** `km-admin`'s server module states the reason: *"four programs in this
product can be open at once and the favicon is what tells two tabs apart."* A tab is found in a strip
of twenty by its icon, at sixteen pixels, and never by the colour of a button inside the page. Both
marks live in the shared crate, the host picks one, and a test asserts the two sets of bytes differ.

**A host's own pages share this stylesheet's 46rem body**, so the review grid reflows to fewer columns
and the eight-column bank table scrolls inside its own section — the answer this stylesheet already
gives for a table on a phone: *"the one thing this layout cannot make narrow, so it scrolls inside
itself rather than making the whole page scroll sideways."* A wider body for one host's pages is a
per-host layout, which is what one stylesheet exists to not have.

## Everything it makes is kept, listed, and sent as a second act

**`km-admin` writes every pack and every bank into its own folder, lists what is in that folder, and
sends one when somebody presses Send.**

**The ordering is the argument**: the file exists before anything is sent, so nothing that has already
been paid for can be lost by something that fails afterwards. **A machine that is off is then a delay
rather than a wasted afternoon.** Fetching a bank is up to a gigabyte over somebody's home connection
and building a pack is a few thousand JPEG decodes; a design that only uploaded would throw both away
because a box under a television was unplugged, or because the password nobody has written down was
set last month.

**Getting a thing and giving it to a machine are not one act**: the first is expensive and happens
once, the second is free and happens as many times as there are machines. Sending automatically at
the end of a download gets two things wrong that only show up in a house rather than in a design — it
chooses the machine an hour before anybody could have, since somebody who picks a second machine
while a gigabyte is coming down has said nothing about where it should go; and it makes a second
machine cost a second download, because there is then no route that sends a file already on the disk.

So the pages list what is on this computer and Send is its own button. **The failure message says
both halves** — *"is still in this program's folder, but sending it failed: …"* — because the point of
writing it down first has not changed. And **a bank already on the disk is not re-fetched**.

**Every build is kept, in a folder of its own.** Building a pack into one directory that the next
build clears destroys the search somebody liked as soon as they try another one — tolerable if a pack
were installed once at the moment it was made, and not once a pack is a thing you keep and hand out.
The zip, its manifest and its attribution travel together; the loose images do not, because the zip
already contains them and a second copy would double what a pack costs on disk.

**And each row can be removed**, which follows from the rest: a folder that only ever grows is a
folder that needs pruning, and the people this program is for are exactly the ones who cannot reach it
with a file manager. Removing takes this computer's copy and nothing else — a machine that was sent a
pack keeps it, because that copy is the machine's now.

**It also makes the program useful with no machine at all**, which was not the aim and is worth
keeping: a pack built here is a zip anybody can drop into a wallpaper folder by hand, and a verified
bank is a `.sf2`. Nothing about either depends on the upload having happened.

## A file you already have is passed through, not kept

**`km-admin` also sends files that were already on the computer it is running on** — a package, a
bank, a photograph — and those are staged in a folder that is emptied when the transfer ends,
whichever way it ended. The heading above says *makes*, and that word is load-bearing.

**The rule above is about work that was paid for.** A gigabyte over somebody's home connection and a
few thousand JPEG decodes are hours that a switched-off machine must not be able to destroy. A file
the owner already had cost nothing to have and is *still where they picked it from*, so the retry
story that argument turns on is already better than a copy here could make it: the original has not
moved. What a second copy would buy is nothing; what it would cost is a duplicate of somebody's
library, growing, in a folder they did not choose and nothing prunes.

**Nor is it listed.** A row in that table is something to send again to another machine; a file
somebody already has is one they can send again from where it is, with the chooser that sent it the
first time. Listing it would mean keeping a copy to list, and Remove would then be a button that
deletes something this program never made.

**So it is staged rather than kept**, and the staging has two halves because one of them cannot
cover a kill. A guard removes the file when the sending task ends — success, refusal or panic — and
the folder is emptied at startup, which is the only moment nothing is passing through. The second
half is not optional: a connection that breaks mid-stream leaves a partial file in the folder for
ever if one failure path is missed, and `bank.rs` has missed one.

**Staged at all, rather than piped straight through**, and that is a lifetime rather than a
preference: axum's multipart `Field` borrows the request, so forwarding it directly would hold the
browser's request open for the whole machine-side transfer — no progress bar, no job, an hour-long
upload timeout applied to a browser, and a failure reported after a gigabyte with nowhere to put it.
Staging also restores the request's `Content-Length`, which is the thing that lets the machine refuse
an oversized upload before reading any of it.

**An upload that waits says it is waiting.** A package runs to two gibibytes and the forward to the
machine is allowed an hour, so a form post that draws nothing on the page cannot be told from a
program that has stopped. What it draws is an indeterminate report: the button greys out and says
what is happening, and a bar sweeps under the form until the page turns over.

**Indeterminate rather than a percentage, because neither number is the one being waited on.** There
are two hops — the browser to this program, over loopback and quick, then this program to the
machine, over the house's Wi-Fi and slow — and only the first can be measured from a browser. A bar
that reached full while the fast half finished and then held still for ten minutes would be a more
confident lie than a sweep. What this does not do is survive a reload: the report belongs to the page
that started the upload, and a page loaded again while one is in flight shows nothing.

## Finding the machine, and remembering which one it was

**`km-admin` lists the machines advertising themselves, and it remembers the one somebody picked.**

**Listing is not setting, and that is not this decision's to make** — it was made in
[`Discovering a machine in the package builder`](curation.md#discovering-a-machine-in-the-package-builder),
which binds every tool in this product that *writes* to a machine. `km-admin` uploads packages,
banks and photographs, so it is on that side and not the offline remote's: the worst thing that
happens to a remote which re-points itself is browsing the wrong catalog, and the worst thing that
happens here is a gigabyte landing on somebody else's television. So the browse waits its whole three
seconds rather than stopping at the first answer — a list showing one of two machines is worse than no
list — every row is a button, and the fragment holds nothing that can fire without a click. A test
asserts that last part, because it is the kind of property a later convenience removes by accident.

**The page does look by itself, in exactly one state.** With no machine chosen there is nothing to
disturb, and the alternative is an empty page asking somebody for an IP address they may not know.
What changes is when the browse happens; what is done with the answer does not change at all.

**What is remembered is what somebody chose, not what answered**, and this is the one place
`km-admin` deliberately breaks step with `km-remote`. That one writes an address down only once the
machine has replied, which is right for a remote — an address that never answers is of no use to it.
It is the wrong rule here, because this program is *built* around the machine being off: the whole
argument of
[`Everything it makes is kept, listed, and sent as a second act`](#everything-it-makes-is-kept-listed-and-sent-as-a-second-act)
is that somebody can point it at an unplugged television box, spend an hour downloading a bank, and
send it tomorrow. Forgetting the address overnight because nothing answered would take that back.

**`--machine` wins for the run and is not written down.** It is `km_remote_core::find::locate`'s
ordering — asked for, then remembered, then the network — minus the last rung, which is the one this
program may not have. A one-off `--machine` for a test must not quietly replace the address somebody
normally uses, and the address on the command line is already the address for that run.

**What is remembered is a record rather than a line.** `machine.json` holds the machine's instance id,
its address, its name and when it last answered. The **id** is what makes a home network's moving
addresses survivable, and it changes nothing about *what somebody chose*: an address is what a person
picks, and only a `/discover` can say which machine is at it, so a chosen machine starts with no id
and gains one the first time it answers. The one writer of a *choice* is `State::set_machine`, whose
only caller is the address form and the rows a look turned up. Following that id to a new address
rewrites the **url** of a machine somebody already chose — see the exception in
[`Discovering a machine in the package builder`](curation.md#discovering-a-machine-in-the-package-builder),
which binds this program too.

**Its own file, `machine.json`, beside `settings.json` and `provider-keys.json` and in neither.**
Those two are already split so that *remember my search terms* and *remember my key* can never become
one decision — see
[`Where a key somebody typed into a page lives`](repository.md#where-a-key-somebody-typed-into-a-page-lives)
— and a machine address is a third thing that is neither. It is also deliberately not in the module
that talks to the machine, whose standing rule is that it writes nothing down: a bearer token there
lives in memory for the life of the process and never reaches a file.

**The Machine tab is first in the nav**, which is the one place this program stops mirroring the
machine's own `/admin/` tab order. Songs, Pictures and Sound are three halves of one errand and read
alike on purpose; Machine is not a fourth errand but the precondition for all three, since every other
page can do nothing but write a file down for later until this one has an address.

## The front door is where a tool is pointed and let in

**`km-admin` opens on a page with no tab strip: the machine it last chose, the machines answering on
this network, a box for an address, and a box for the password. Every launch, and the tabs are not
drawn until a machine is picked.**

**The two boxes are one form because they are one errand.** Nothing behind this page can write to a
machine until it has been told which machine and been given that machine's password, so splitting
them across two pages makes the second one somewhere a person has to be sent. It is also why a write
refused for want of a password comes back *here* from whichever tab it was made on: a message naming
the fix on a page that does not carry it is a message somebody then has to go and find.

**A control that refuses says so on a page, and the page is the one that can mend it.** Every control
on these surfaces is an ordinary form, so the browser navigates and a status with a sentence in it
becomes the whole document: an unstyled line of text, no heading, no strip, and no way back. The
answer is a redirect carrying `?kind=&said=`, which lands on a drawn page with a banner on it. The
exception is a fragment asked for by a script, which keeps its status because the script is what
words it, and a redirect there would swap a whole document into a corner of the page.

**It opens every launch, with the remembered machine already selected.** The alternative — straight
to the tabs whenever a machine is remembered — saves one press and gives up the thing this page is
for: a house with a machine under the television and another on a desk gets an answer to *which one
is this about* before anything is sent, rather than after. The scan runs on every one of those
launches too, so a machine that has moved is visible before a gigabyte is aimed at where it used to
be.

**Listing is still not setting, and on a page of radios that needs saying precisely: the only row
that opens selected is the machine somebody already chose.** A discovered row is pressed twice — once
to pick it, once to submit. That is
[`Discovering a machine in the package builder`](curation.md#discovering-a-machine-in-the-package-builder)
unchanged, and a pre-checked row would be adoption with a coat on. A test asserts the fragment the
browse fills carries no `checked` at all.

**The door does not open without a password the machine accepted.** Every control on every tab behind
it writes to a machine, so a program let in without one refuses each press in turn, three screens from
the box that would fix it. That reads as a broken machine rather than as a question nobody answered,
and it is the failure the whole page exists to prevent: the point of asking *which machine* before
anything is sent is to have the answer before it matters.

**The page says which of three positions it is in, and asks for a password only in the third.** A
password saved on this computer, a token already bought this run, or neither. The first two are facts
the door can state without asking the machine anything, and each of them is a way in — so the box
sits behind *Type a different password* where there is one, and is the whole of what is drawn where
there is not. A box offered on its own asks for something this program is holding, and a page that
does that reads as not having heard the answer.

**Blank means the way in this program already has.** Somebody who ticked the box does not type it
again every launch, and neither does somebody who logged in ten minutes ago and came back from a tab:
a token the machine issued is a password it accepted, which is the whole of what this door asks for.
Somebody with neither is turned away here.

**The sentence names the machine, and stops claiming to when another row is picked.** *Which machine*
and *which password* are one errand, so a page whose subject is choosing between machines cannot
answer the second with the word *this*.

**The tick rides with a typed password, and *Forget it* is the way out.** The checkbox is a statement
about what this computer should be remembering and it is spent on the password in hand, so a pass
that types none leaves the store alone: an entry that forgot what it had just used would be the door
deleting a credential as a side effect of opening. The button that means that sits beside the
sentence saying there is something to forget, which is where somebody looking for it is already
reading.

**What it costs is entering a machine that is switched off**, and that is a real loss rather than a
tidy-up. A password can only be checked by the machine that set it, so requiring one requires the
machine. Pointing this at an unplugged television box and configuring it before it is plugged in
stops working;
[`Everything it makes is kept, listed, and sent as a second act`](#everything-it-makes-is-kept-listed-and-sent-as-a-second-act)
is unaffected, because what that decision protects happens *after* a machine has been entered — a
bank downloaded today is still sent tomorrow, to a box that was off in between.

**A machine that did not answer is told apart from a password it refused.** One is switched off and
the other is mistyped, and they are two different things to do next.

**Drawing the page asks the machine nothing, which is what makes it appear at once.** The ordinary
chrome reads the machine's name, its factory-password flag and its problem count, and over HTTP a
machine that is not answering charges the full ask timeout for each — on the one page that is drawn
before anything is known about the machine, and the one somebody lands on when a write has just been
refused. So the door wears the shared `<head>` and stylesheet and none of the chrome, and draws from
`machine.json` alone. Pressing its button is the other half and does talk to the machine: that is
where the password is checked. The browse arrives underneath as a fragment,
because it waits its whole three seconds by the rule above and a page must not.

**`--machine` still wins for the run and is still not written down**, which this page had to be
careful about rather than inherit: it pre-selects whatever address the run holds, and under
`--machine` that is the command line's. So entering on the address already held writes no address
down. That also keeps a re-entry after a refused write from throwing the held token away with the
client.

**A remembered password is keyed to the machine in force**, which is the same rule read carefully: a
run pointed somewhere the record does not name has no identity to key one under, so it neither reads
one nor offers a box to store one. Without that, a `--machine` run would take the id out of a record
about a different box and hand that box's password to this one.

**The Home page it replaces is gone rather than moved.** Its three cards linked to Songs, to the
picture search and to the bank table, all of which the strip and the Pictures and Sound tabs already
reach; a front door whose job is *pick a machine* has no room for a second one whose job is *here are
the tabs*.

### …and the third question it asks is what language it is in

**A card below the door, and it is this program's own language rather than any machine's.** The
picker names each language in itself, the choice goes in the `km_locale` cookie, and the page that
lands afterwards is already in it.

**A program on an origin of its own needs its own picker, and that is what makes this different from
the machine's `/admin/`.** Both surfaces read the same cookie. On the machine that is enough: the
singer's remote is mounted at `/` and the owner's pages at `/admin/`, one origin, so a viewer who
chose Portuguese on the remote meets Portuguese on both — which is what
[`A viewer chooses the remote's language`](remotes.md#a-viewer-chooses-the-remotes-language-and-the-machine-does-not-choose-it-for-them)
already buys. `km-admin` is a fourth program on a loopback port with no remote beside it, so nothing
on that origin could ever write that cookie: its pages followed `Accept-Language` and there was no
way to disagree with the browser.

**On the door because the door is the page about this program.** Every tab is a page about a
machine, and this is the one question here that no machine has an answer to — beside *which machine*
and *which password*, which are the other two things this program needs told before it is useful.
It is also the page that draws with nothing on the network answering, which is when somebody who
cannot read this program is still looking at it.

**The cookie is spelled once, in `km-locale`, name and path and life together.** Two programs write
it now, and a `Path` that was not `/` or a life that differed would be one surface forgetting what
the other remembered — the same argument that put the name there rather than in either pages crate.

**The heading says which of two languages it means**, and so does the *Screen language* pane on the
*This machine* tab. That one is the television's, in a room; this one is this browser's, on this
computer. Confusing the two is the likeliest misreading of either, which is why neither heading is
just *Language*. The confirmation is worded in the language just chosen, so somebody who picked the
wrong one finds out at once.

## The banks already on this computer are at the top of the list

**`km-admin`'s Sound table sorts what it has already downloaded above what it has not, and does
nothing else to the order.** Underneath, the order is still `km_banks::catalog()`'s — the nine
shortlisted banks in rank order, then the rest by ascending loudness spread — which is
[`Which banks the machine offers`](repository.md#which-banks-the-machine-offers) and is pinned by a
test in that crate.

**The rows somebody presses more than once are exactly the downloaded ones.** Get is pressed once per
bank ever; Send and Remove are pressed once per machine, per bank, forever. Those sat wherever the
catalog put them, so a table whose whole point is that a download is kept made you hunt for the same
row every time you used it.

**Stable.** A sort that also reordered inside the two groups would discard the ranking silently —
the page would still look sorted, and the recommendation would just no longer be at the top of the
banks you do not have.

**It is decided from the disk, not from a record.** `here` is `landed_bank` asking whether the file is
there, the same way [`Choosing a bank`](audio.md#choosing-a-bank) reads its folder every time, so a
bank somebody deleted by hand drops back down the list and one they copied in by hand rises. And it is
independent of whether the machine has it: *here but not sent* is the state the Send button exists
for, and *here and sent* is the one that says a second machine costs no download.

## The machine page's settings are tabs once there is more than one

**`km-admin`'s *This machine* page puts its four errands — Debugging, Its name, Its password, Demo
mode — behind a tab strip under the information panel.**

**Which machine this is, and whether this program is let in to it, are one question and live together
in the information panel.** The panel names the machine, lists its addresses and counts its songs;
under those facts, on the tool alone, sits the sentence saying whether it holds a token and a link to
the page that buys one. **A tab would put that behind a click that is unavailable exactly when it is
wanted**: a machine that has gone away leaves every fact in the panel blank and every errand below it
unanswerable, and that is the state somebody is in when they need the way out.

**Neither the address box nor the password field is on this page at all**, and that is
[`The front door is where a tool is pointed and let in`](#the-front-door-is-where-a-tool-is-pointed-and-let-in)
rather than a change of mind about the paragraph above. The argument there was that a control which
gets somebody out of a dead page may not be behind a tab; a page with no strip and no chrome honors
it further than a panel could, because that page draws correctly when nothing answers and this one
does not.

**The machine's own `/admin/` draws none of it**, which is `choose_machine`: a machine has no say
over which machine it is and holds no session with itself, so the panel there is facts and nothing
more.

**`/admin/`'s *This machine* tab does the same, and did not at first.** It carried seven stacked
cards, which is the arrangement this rule was written against — and the reason it went second rather
than first is only that the strip was built here. Its panes are the same, in the same order, under
the same names, which is
[`Two admin surfaces, one vocabulary`](#two-admin-surfaces-one-vocabulary) rather than a coincidence:
Debugging, Its name, Its password, Demo mode, and then **Screen language**, which both surfaces draw.
*Sign out everywhere* shares the password pane rather than taking one of its own — two different
acts, one errand, and a tab names an errand.

**Three switches share the Debugging pane on both surfaces, and two of them are one condition.**
`/dev/` and its passwordless API need the console's switch *and* debugging, so a pane that separated
them would let somebody turn one on, see nothing happen, and have nowhere to find out why — both
surfaces say which switch is missing rather than only reporting success. The frame-statistics switch
is there because all three are the same kind of thing: something a machine in a living room should
not have on. **It is also the only one of the three that needs no restart**, and both pages say so,
because it mounts nothing — the next frame draws it.

**Every switch on that pane reads the value the machine has *written down*, not the one it is
running.** Debugging and the console take effect at the next start, and the running value is a
snapshot taken when the router was built — so a switch drawn from it read *Turn debugging on* both
before the press and after it. `km-admin` reads `GET /debug` as well as `/discover` for this: that
call reports the running value only, and had been the switch's only source.

**The information panel is above the strip and never in it.** It is built entirely from public reads
and is what the page is *for*; the five below it are things you came to change, and you came to change
one of them. Stacked, changing a password meant scrolling past a debug switch and a rename box, and
the page's length was the only thing saying how much there was to get wrong here.

**Flat, logged out, because one section is not a strip.** Four of the five need a token, so what is
left is the address form — and a tab strip holding a single control is furniture around it.

**The strip is `km-package-builder`'s `.curate` pattern**, radio inputs and `:checked ~`, no
JavaScript, and that program's stylesheet already said it was built to match this one's nav so an
in-page strip and a page-level one read as the same kind of control.

**One page set draws this strip for both admin surfaces**, which is what
[`One page set behind two hosts`](#and-one-page-set-behind-two-hosts) is for. A strip built in one
program and reproduced in the other is the drift that entry exists to stop, and the reason offered
for accepting it — *"`tools/cmd/assets/` is a second cargo workspace and each program compiles its
own `static/` in"* — describes the arrangement rather than arguing for it: the workspace boundary
blocks an HTTP client crossing, not a page crate.

**`km-package-builder`'s is a copy, and that one stands**: that program is on the far side of the
same boundary *and* has no admin page to share, so what it borrows is a pattern rather than markup —
the same boundary
[`A wallpaper pack is named`](interface.md#a-wallpaper-pack-is-named-and-the-name-is-a-label-rather-than-part-of-the-search)
argues about a slug function.

**Two properties of it are load-bearing rather than stylistic.** The radios are *flat siblings* of the
panes, because a strip that wrapped each pane would need `:has()` to reach sideways — and the rule
hiding a pane is unconditional, so a browser that could not evaluate that would show no pane at all
and lose this page's whole settings half. And they sit *outside every form*, because there are five
`POST` forms below them and a radio inside one would post a key nobody reads. A test asserts the
second by position.

**The pane a save was made on is the pane that comes back.** Every one of these is a `POST` that
reloads the page with a notice at the top of it, and a notice about a rename read underneath a debug
switch is a notice about a page somebody is no longer looking at. The pane rides in the redirect's
query string, is checked against a fixed list of names, and an unknown one opens Debugging — which is
the pane that is never empty, Demo mode drawing a sentence instead of a switch when the machine did
not answer.

**A pane a host does not draw opens Debugging too**, and that is the load-bearing half rather than a
courtesy: the rule hiding a pane is unconditional, so a name whose radio is absent would leave *no*
pane open and take this tab's whole settings half with it.

**On `km-admin` the *Log in* pane opens instead while that program holds no token**, because until it
does, nothing else on the page can be saved at all.

## `km-admin` says logged in

**One state, one verb, and the verb is *log in*.** The field is `logged_in`, the method is `log_in`,
the capability is `log_in`, and `km-api` answers a stale token with *log in again*. Three spellings
of one idea on one page is a page that reads as though it is describing three, so the prose uses this
one too.

**`POST /admin/login` is the route, and both surfaces share it.** What differs is where the token
lands — a cookie for the browser on the machine, this program's own client in a tool — and that is
`guard::LoggedIn`'s two variants rather than two routes. A second path would be a second thing to
keep gated, worded and rate-limited alike.

**Where a tool puts the box is its front door**, which the machine's own page does not have: that
page is behind the password already. A write refused for want of one comes back there from whichever
tab it was made on. A program that cannot be given the password is a program whose every control
refuses, so it is the errand that comes before every other — which is
[`The front door is where a tool is pointed and let in`](#the-front-door-is-where-a-tool-is-pointed-and-let-in),
and the reason the box is on that page rather than on a tab.

**What stays on the Machine tab is the *state*, not the field.** *This program is logged in* — or is
not, with a link to the door — sits in the information panel beside the machine's name, because
*which machine* and *are we let in to it* are two halves of one fact about a connection.

*This program is logged in* names the subject, which is what a status sentence has to do. A sentence
saying a door opened without saying who opened it, when, or what would close it leaves the reader
asking whether it means this program, this browser, or the machine.

**This does not reopen [`Two admin surfaces, one vocabulary`](#two-admin-surfaces-one-vocabulary).**
That rule is about the two programs naming the same *page* and the same *state of a machine* alike —
a tab called *This machine*, a warning about a factory password — because those are things a person
carries from one surface to the other. It is not a claim that every verb inside each program must
match: `/admin/`'s `sign-in` key is a button on a form that only exists there, phrased for whoever
is reading the machine's own page, and changing an i18n key in both catalogs to bring a button into
line with a status sentence in a different program would be spending a translation on tidiness. If
the two ever describe *the same state* in two ways, that is the rule being broken and this paragraph
is not a licence for it.

## The appliance's power button is a deploy decision, not an install one

The `.deb` carries the logind drop-in that makes a single press of the box's power button shut the
machine down, and **ships it inert under `/opt/karaokemachine/` rather than into
`/etc/systemd/logind.conf.d/`**. `tools/platform/linux/deploy.sh` is what copies it into place.

**That is the same position as
[`What the machine is, on Linux`](#what-the-machine-is-on-linux) takes about the unit**, with one
difference in the mechanism that matters. A unit file on disk is inert until something enables it,
so the package can ship it and leave the enabling to a deploy; a file under `logind.conf.d/` is live
the moment it exists, so there is no shipped-but-off state for it to be in. The faithful analogue is
to ship the *text* and let the deploy install it. Repurposing a desktop's power button must not be a
consequence of `apt-get install`, exactly as seizing tty1 must not be.

**What it pins is systemd's own default.** `HandlePowerKey=poweroff` is what a stock Debian already
does, so on a box that has only ever been an appliance the file changes nothing. It defends against
the population `deploy.sh` already warns about for display managers: a box repurposed from a desktop,
where a desktop environment's package set this to `ignore` or `suspend`. On a machine under a
television that is the one control there is, and it would silently do nothing.

**A long press is left to the firmware.** `HandlePowerKeyLongPress` stays unset, so holding the
button is still the hard cut somebody reaches for when the machine has stopped answering — the one
case a software handler cannot serve.

**The deploy reports rather than restarts.** logind has no reload, and restarting it underneath a
PAM session that is holding DRM master is a real risk for no gain when the file restates the default
anyway; so the deploy prints what logind believes now and says the file applies at the next boot. It
also warns when `acpid` is enabled, which is the likeliest reason a press does the wrong thing and is
invisible unless something looks — the same species of check as the display-manager warning beside it.

**Shutting down shells out to `systemctl` rather than taking a D-Bus dependency.** That is a
dependency decision and belongs here. `systemctl poweroff` calls
`org.freedesktop.login1.Manager.PowerOff` on the system bus — the same method a client would, checked
against the same polkit action, with the session resolved from the child's PID, which inherits the
service's cgroup. A `zbus` client would buy a typed error at the price of some twenty crates in a
workspace with no D-Bus stack at all, for one call made at most once per process lifetime. And the
sentence `systemctl` prints on stderr is worth more than the typed error would be: it is the
diagnosis. `km-osopen` reaches `xdg-open` the same way, for the same reason.

**`TimeoutStopSec=30s` is part of the same change.** Unset, the default is ninety seconds — and a
power button that leaves the television lit for a minute and a half reads as a button that did not
work, which is the thing somebody then holds down. See
[`docs/architecture/appliance.md`](../architecture/appliance.md) for what is in that budget.

## What a release page says, and to whom

**A GitHub release page is read by somebody choosing a file to download, so it is written in the
register [`What a user reads is written in plain application language`](foundations.md#what-a-user-reads-is-written-in-plain-application-language)
gives an operator**: a sentence or two, consequence first. It names each asset and what that asset
is for, says what the platform will do about an unsigned build, and gives the steps a reader
follows.

The reasoning behind any of it belongs in `docs/` and in the comments beside the code. A person
reading a release page has already decided to try this, and every sentence that argues with them is
one standing between them and the download.

**The body is `tools/dist/release-notes.md`**, tracked for a mechanical reason: `check-prose.sh` and
`check-no-local-refs.sh` both choose what to read through `git ls-files`, so a body typed at the
point of upload is read by neither, and a release page is exactly where a local drive name would
travel furthest. `tools/dist/release.sh` substitutes this run's version and the carol pack's name
into it, so the tracked file states no version of its own.

**A release page names the assets it carries, and nothing else about the tree.** The
`chore(release):` commit body is written for somebody who has the source in front of them and names
files that mean nothing to a reader who has a download.

### An asset's name says which system it is for

**Every file on the page names its system in the filename, ahead of the architecture**:
`karaokemachine-setup-<version>-windows-x86_64.exe`,
`karaokemachine-setup-<version>-macos-<arch>.pkg`, `karaokemachine-<version>-android.apk`,
`karaokemachine-<version>-ios-unsigned.ipa`, and a `.deb` and a tarball that carry Debian's own
conventions. **A release page is flat**, so the folder that says it in `dist/setup/windows/` and
`dist/setup/macos/` is not there, and a reader choosing between twelve files has the filename and
the sentence beside it.

**The extension is not enough on its own.** A `.pkg` and an `.exe` each belong to one system, and
somebody who knows that reads the page in a moment; somebody looking at a list of downloads for the
first time is being asked to know it. The cost of saying it is seven characters in a name.

**The build names its own product, and the gathering step copies it.** `release_rows()` in
`tools/dist/release.sh` renames only where a build cannot name its file — both Android projects
produce `app-release.apk`, and two assets cannot share a name. Everything else travels under the
name the build gave it, so one name holds in `dist/`, in the documents and on the page.

## A release page carries the platforms the machine cutting it can build

**`tools/dist/release.sh --platforms windows,linux,android` names what a cut carries**, and the rows
for every other platform leave the table, the count and the body's download table together. A run
that names none carries all twelve, which is the full release and the default.

**No one machine builds all twelve.** The two `.pkg` files and the two `.ipa` files are produced on a
Mac and the rest are not, so a machine without one has four carriers it cannot stage and a refusal
it can do nothing about. Holding a release until every platform can be built on one computer waits
on hardware rather than on the software being ready.

**A platform that is named and not staged still stops the run.** That refusal is what makes a
gathering step worth having, so `--platforms` narrows what is asked for rather than softening the
answer. The four fields beside a skipped row describe a file the run was never going to look for,
where a missing carrier of a named platform is one somebody meant to build.

**A page never names a download it does not have**, a row that downloads nothing being worse for
somebody choosing a file than a platform the page says nothing about. So the body drops with the
carrier, in two shapes and neither a second copy of the table:

- **A download table row goes when its filename matches a skipped row's pattern.** `release_rows()`
  stays the one place a published name is written down, and matching against it keeps a name in the
  body belonging to no carrier at all an error rather than a row quietly removed.
- **Prose goes when it sits between `<!-- platform: <name> -->` and `<!-- /platform -->`.** A
  Markdown comment renders nowhere, so `tools/dist/release-notes.md` reads as the whole page to
  whoever edits it, and a section written for a platform later is dropped by its own marker rather
  than by the script learning its heading.

**The body names what the run gathered, so one machine writes the page.** A second machine adding
its carriers to the same draft uploads them and leaves the body alone; a second run of the gathering
step rewrites the body to its own platforms and takes the rows the first one wrote with it.
