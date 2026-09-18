# The appliance

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

A Debian box under a television, coming up by itself on boot, controlled from phones on the LAN.
`tools/platform/linux/deploy.sh [user@]host` builds the package, sends it, installs it and enables the
service; `task deploy:linux HOST=user@box` is the same thing through the index.

**There is no display server, and that is the point.** SDL3's `kmsdrm` backend draws straight to
DRM/KMS from a virtual terminal — no X, no Wayland compositor, no display manager, no desktop session.
For an appliance this removes a whole layer that can fail, start slowly, or draw something over the
lyrics.

## DRM master needs a seat

SDL's kmsdrm backend opens `/dev/dri/card0` and calls `drmSetMaster`, which succeeds only for root or
for a process belonging to a **logind session on the active VT**. An ordinary service unit with
`User=` has neither — and because the machine treats a display that will not start as non-fatal, the
symptom is a black television from a unit that reports `active (running)` and never exits nonzero.

```ini
PAMName=login
TTYPath=/dev/tty1
StandardInput=tty-fail
Conflicts=getty@tty1.service
```

`PAMName=login` runs `pam_systemd`, which registers a session against `seat0` — that is what grants
the `uaccess` ACLs on `/dev/dri/*` and `/dev/input/event*` and sets `XDG_RUNTIME_DIR`. This is the
construction the `cage` project documents for its own unit, which is the signal that it is the trodden
path rather than a trick. `cage` itself is the documented fallback, written as commented lines
directly beneath the live ones so switching is a visible edit rather than a rediscovery.

**`TTYReset` and `TTYVHangup` are deliberately absent**, and copying `cage`'s unit puts them back.
Both reset the virtual terminal around the service, and resetting a VT repaints the console — which
wipes the boot splash a fifth of a second after `--retain-splash` has gone to the trouble of leaving
it there. Neither is load-bearing for the seat: what `TTYVHangup` buys is a clean restart against a
process holding tty1, and `Conflicts=getty@tty1.service` is what actually keeps tty1 free.

**The package ships the unit disabled.** A unit file on disk is inert; enabling it is what seizes
tty1, and that should not be a consequence of installing an application on somebody's desktop.
`deploy.sh` enables it — that is the step that turns a machine into an appliance. `postinst` *does*
restart the service when it finds it already enabled, so installing a newer `.deb` by hand does the
obvious thing.

**`prerm`, not `postrm`, stops and disables.** dpkg removes the package's files *between* the two, so
by `postrm` the unit file is gone and `systemctl disable` has nothing to work from — it would leave a
dangling symlink in `multi-user.target.wants` and the next install would come up enabled without
anyone asking.

## The `karaoke` account

`postinst` creates a system user with home `/var/lib/karaoke`, in `video`, `input`, `audio` and
`render`. Settings and the catalog then resolve through XDG below that home, which keeps the
config/data split XDG makes and one `--data-dir` flattens.

**The tempting alternative — `--data-dir /var/lib/karaoke` — was a trap**, and it was never only the
appliance's problem: `rooted_at` moved the *asset* directory too, so the machine stopped finding
`/opt/karaokemachine/assets` and came up on its sine test tone with nothing saying why. Since the flag
is documented as "keep a run out of the real install", every developer using it had a machine with no
SoundFont, no font and no wallpapers. **Assets are not data** — they ship with the build rather than
accumulating with use — so the command line now takes a `data_rooted_at` that roots config and data
while taking the asset directory from ordinary discovery. `rooted_at` is unchanged and still means one
directory for everything, which is what tests and a portable install want.

The group memberships are belt and braces beside the seat: logind's ACLs already cover the service,
and these keep a manual `sudo -u karaoke -H karaokemachine` working for debugging — `-H` included,
because without it sudo leaves `HOME` pointing at the calling user and `--show-paths` then answers a
different question than the one being asked.

**`useradd`, not `adduser`.** The idiomatic Debian call is the wrong one: `adduser` stopped being
essential in bookworm, so a package whose maintainer scripts use it must Depend on it — and
`debian:13-slim`, which the verifier installs into, does not have it. Taking a dependency to create
one account is the wrong trade when `useradd` comes from `passwd`, which is Priority: required. The
group loop is guarded by `getent group`, because `input` and `render` exist on a real systemd machine
and not in a container.

## Three dependency defects the appliance found

All the same species as `$auto` being insufficient — things SDL `dlopen`s, which `dpkg-shlibdeps`
cannot see:

- **`fonts-dejavu-core`, and it is not optional.** Finding no font is an error that goes down the same
  non-fatal path as the display, so on a minimal Debian the screen simply stays black. A plain
  packaging bug that was never specific to the appliance.
- **The kmsdrm libraries** — `libdrm2`, `libgbm1`, `libegl1`, `libgles2`, `libgl1-mesa-dri`. The last
  is the driver that actually talks to the hardware; without it EGL finds only software rasterisation.
  `libegl1` is **promoted out of Recommends**: optional for X11, mandatory here.
- **`libudev1`**, for SDL's input enumeration and hotplug. On every systemd machine already — but
  "already there in practice" is precisely what shlibdeps cannot say on our behalf.

**What the box does *not* get is a display-server stack, and that took the package carrying its own
ffmpeg.** Every X11 and Wayland library is a `Recommends`, so `deploy.sh`'s `--no-install-recommends`
leaves a box that cannot run SDL's x11 driver at all — see
[`An appliance install carries no display-server stack`](../decisions/distribution.md#an-appliance-install-carries-no-display-server-stack).
That was unreachable while the package linked Debian's ffmpeg: `libavutil` has `libX11` as a direct
`NEEDED`, beside `libva-x11`, `libvdpau` and `libOpenCL`, so the appliance loaded an X client library
and two video-acceleration stacks it can never call.

**`libX11` and the `libxcb` family are still there, through Mesa**, whose EGL and gallium packages
hard-depend on `libx11-xcb1` — and Mesa is what kmsdrm draws through, so declining them means
declining the picture. The eight libraries SDL's x11 driver opens are what is absent, which is why
the verifier asserts those rather than counting `libX11`.

**`libstdc++6` arrives with the bundling and is the one entry nothing could have derived** —
`libopenh264` is C++ where the rest of that directory is C, and the library needing it is one the
package carries rather than one the binary links.

**`deploy.sh` warns about an enabled display manager.** gdm/lightdm/sddm hold DRM master for
themselves, and kmsdrm cannot then become master. That is the most likely reason for a black
television on a box repurposed from a desktop, and it is invisible unless something looks.

## What a deploy assumed and the box would not answer

Three faults of one species — the script assumed a privilege or a visibility it does not have, and
each failed in a way that looked like something else. **None could have been caught from Windows, and
two were only reachable after the one in front of them was fixed**, which is the argument for
deploying against a real locked-down box rather than a permissive one.

- **`sudo -u karaoke -H … --show-paths`** was how it asked where the packages folder is. A box whose
  sudoers lists the specific commands a deploy runs — the *correct* way to set one up — does not permit
  `sudo -u karaoke`, so that line printed `(unavailable)` on every deploy that ever ran and `--songs`
  had never worked at all. It is `HOME=$(getent passwd karaoke | cut -d: -f6) karaokemachine
  --show-paths` now, run as the calling user: `--show-paths` derives everything from `HOME` and reads
  nothing, so this gives the service's answer with **no privilege whatsoever**, and `getent` keeps it
  the package's answer rather than the script's guess at it.
- **The registration loop globbed the destination.** `for f in "$dest"/*.kmpkg` runs in the deploying
  user's shell and the directory is 0700 owned by `karaoke`, so the glob matched nothing, stayed
  literal, and posted a package path of `*.kmpkg` — earning a **400 that reads exactly like an API
  fault**. Fixed by recording the names before the move; **the loop is gone entirely now**, for the
  reason below.
- **`--songs` sent no video**, because both transfer branches filtered to `*.kmpkg` and video then
  lived in a folder beside the package. The result installs cleanly, lists its video songs, and each
  one is a missing-media skip at the moment somebody queues it. Later reversed at the source by moving
  media inside the `.kmpkg`, which makes a bug of this shape unwritable — kept as the clearest
  example of what a pairing rule costs: **the fault was in a filter list, three files away from the
  rule it broke.**

**Then the route the loop posted to moved, and the loop went with it.** Installing a package became an
owner's act — `POST /api/v1/packages` is `POST /api/v1/admin/packages` now, and only the `GET` stayed
behind at the public path. So the deploy's registration call earned a **405 on every package**, printed
`failed` beside each one, and **still exited 0**; the songs sat in the packages folder unread until
something else restarted the machine, and the box showed an empty catalog after a deploy that said it
had sent them. The fix is not a token but a restart: **the folder is what says what is installed**, so
reading it again is all the registration ever achieved, and a restart needs no credential — which
matters because the deploying account cannot read the machine's settings file to find one, the data
directory being 0700 and owned by `karaoke`.

Three distinct ways `--songs` has claimed success while delivering nothing share one shape: **every
one is a step that can fail without failing the script.** A loop that prints `failed` per item and
returns 0 is the same fault as `apt` printing `already the newest version`.

**`apt-get install ./x.deb` compares versions and nothing else**, so handed a package whose version is
already installed it prints `already the newest version` and does nothing. The version changes at a
*release*, not at a commit, so in development the ordinary case is a deploy that copies 31 MiB,
restarts the service, prints a healthy report with the right version in it, and ships **none of the
code just built**.

So it is `--reinstall`, and the deploy prints the installed binary's hash — because the one thing a
deploy is for is that the binary on the box is the binary you built. The symptom is indistinguishable
from a build problem, so it gets chased as a stale cargo cache, then a log-level filter, then a
tracing target, each of which *could* cause it.

**A second, unrelated staleness was in series with it.** `cargo` inside the build container decides
what to recompile from mtimes on the bind-mounted source tree, and edits made on the Windows host do
not reliably advance them — a build that should have recompiled two crates reported `Finished in
9.78s` having compiled nothing. Worth knowing that **two different silent-staleness failures were in
series**, because fixing the first alone changes nothing visible and reads as the fix not working.

## A shipped default that reached only a machine which had never been started

**The mechanism this happened to is gone — the per-endpoint access list — and the finding is the
reason to keep the section.** Choosing the machine's audio output shipped `admin`, the single route
on that surface that was not public. `Acl::access` fell back to `default_access` for a route the map
did not mention, and its own documentation said that fallback was "what makes a new route's default
reach a machine that already has a settings file". It did not: **`Acl::fill_missing` ran first on
load** and inserted `Public` for every absent route, pinning the wrong value into `settings.json`
before `access` was ever consulted. So the admin default applied to a fresh install and to nothing
else — and every machine that has ever been switched on has a settings file.

**Two tests can cover this area and miss it the same way.** One asserted the shipped defaults through
`Settings::default()`, which never called `fill_missing`; the other called `fill_missing` and then
asserted the route *count* and that a pre-existing entry kept its value — never what a newly filled
route was set to. **A test that checks what arrived and not what it is worth is the shape to be
suspicious of.** That is the transferable part, and it is why the tests written for what replaced
this drive a *restart* rather than asserting on the API's own answer.

**A permission cannot go stale this way.** It is the URL prefix a route is declared under,
which no settings file can be out of step with. What can still go stale is anything a settings file
*does* hold, which is why `settings_version` exists: a file written in an older shape is set aside,
with both numbers in the log, rather than read as if it meant what the current shape means.

## Three things a real screen found

**The journal is filed under the session scope, not the service.** `journalctl -u karaokemachine`
returns the two lines *systemd* wrote and none the machine wrote — a direct consequence of
`PAMName=login`, which puts the process in a session scope. **Use `journalctl -t karaokemachine`.**
The failure is nasty because it reads exactly like an application that started and then said nothing,
which is also what a real hang looks like: the very thing that makes the screen work is what hides the
log.

**Fullscreen was setting a 1280×720 mode on whatever panel it found.** The window was created at the
*windowed* size, and on kmsdrm there is no desktop for "fullscreen desktop" to mean anything, so SDL
sets a video mode near the window's. **On a 16:9 screen this is invisible in shape and merely soft**,
which is why it survived every desktop test — a 4K television would have been driven at 720p and
nothing would have looked *wrong*. The window is sized from the connector's preferred mode when
`fullscreen` is set.

**The display chooses the mode, and the machine takes what it is given** — the
[`Any screen size, and the screen chooses the mode`](../decisions/distribution.md#any-screen-size-and-the-screen-chooses-the-mode)
decision. Every size is supported and none is privileged, so nothing here may assume 16:9: DCI 4K is
4096×2160 against UHD's 3840×2160, and the two differ in width alone. Both are in the `SCREENS`
matrix the geometry tests loop over, which spans 0.45 to 2.37 in aspect ratio precisely so a layout
cannot pass by being right about one screen.

**That fix was only half the trip, and the other half broke `F`.** SDL restores a window to the size
it was *created* at — which the fix had just made the whole display. So leaving fullscreen gave back a
window exactly the size of the panel: the key worked and changed nothing anybody could see, the only
difference being the cursor reappearing. **The outward trip wants the display's mode and the return
trip wants the configured size; they are two different numbers, and the first fix used one for both.**
`apply_fullscreen` takes the size to come back to and calls `SDL_SyncWindow` first, because the
fullscreen change is asynchronous wherever a window manager has to agree and a size set before it
lands is overwritten.

**Auto-repeat was the second way `F` could look dead.** Holding the key toggled fullscreen at the
repeat rate and stopped on whichever parity the release gave. Held Escape or BACK was worse — the
repeats walk past the leave-fullscreen step into the exit. Repeats are dropped for those three and
nothing else, because every other binding is either idempotent or worth repeating.

## A save-on-shutdown that never runs

Settings are saved on shutdown, so editing `settings.json` under a running machine is useless — and
under systemd the save half needs SIGTERM handled explicitly. `wait_for_signal` selecting on
`ctrl_c()` alone catches SIGINT only, so systemd's SIGTERM takes the default disposition, the process
dies where it stands and `persist()` never runs.

**That appliance persists no setting at all** — not the transpose, not the volume, not the lyric
offset the whole calibration exercise depends on. Editing the file under a running
machine was in fact *safe*, and everything set from a phone was lost at the next restart, which is the
failure nobody had looked for because the documentation asserted the opposite one.

`wait_for_signal` waits on SIGTERM too, with both futures built once outside the loop rather than per
pass — tokio's handler is process-wide and permanent, but a *stream* that does not exist when the
signal arrives never learns of it, so rebuilding each pass would leave a gap the width of the poll
interval.

**There is no unit test, deliberately.** Proving it means raising a real SIGTERM at the test process,
and a signal is process-wide: raise it before the stream is registered and the default disposition
takes the whole harness down; raise it at all and any other test holding a signal stream sees it. A
test that can kill the suite on a timing edge is a worse trade than the coverage is worth.

**A task spawned beside the runtime hears the stop, whatever the screen is doing**, and it is what
the appliance's own stop goes through. `wait_for_signal_for` is reached only where there is no
display — the `DisplayError::Failed` arm and the display-retry arm — so on an ordinary appliance boot
the main thread sits inside `display::run` for the life of the process and that wait is never
entered. The task sets the same `shutdown` flag the display loop tests once a frame, the power
controls set, and the shutdown block reads, so every route in ends in one place.

**SDL stops it as well, and that is not the thing to rely on.** `sdl.video()` pulls in
`SDL_INIT_EVENTS`, whose quit subsystem installs SIGINT and SIGTERM handlers that post
`SDL_EVENT_QUIT`; the frame loop breaks on that. Two properties make it the weaker of the two.
`SDL_HINT_NO_SIGNAL_HANDLERS` switches it off, so a hint set for any other reason takes the
save-on-shutdown with it. And the event is only seen **when the loop next polls** — a frame blocked
on a seek or on an audio device that will not write is a stop nobody hears until `TimeoutStopSec`
runs out and systemd kills the process with `persist()` never reached. Seen once on an appliance
during a package upgrade: `Stopping`, then `State 'stop-sigterm' timed out. Killing.` thirty seconds
later, with nothing logged in between. It has not reproduced on an idle box.

The flag does not rescue a loop that is genuinely wedged — nothing in the process can — but it is the
half that does not depend on a library's courtesy, and it covers every phase rather than the two the
wait covers.

## Powering the box off, and restarting the application

**Four doors and two destinations** — the physical power button, the owner's page's *Shut down*, its
*Restart*, and `Ctrl+Q` — and **none of them is a second shutdown sequence.**

**Powering off asks logind and stops nothing itself.** systemd then stops the unit exactly as it
would for any other reason, which runs the one shutdown block there is. Setting the machine's own
flag *as well* would race it: the process would begin persisting and dropping the audio device while
systemd was separately stopping it, and the two orderings would interleave differently every time.
That is why the API handler and the `Ctrl+Q` arm both ask and then `continue` rather than breaking
their loop.

**Restarting is an exit.** `Restart=always` restarts after any exit and not after a `systemctl stop`,
so a clean `Ok(())` out of `run` already *is* "restart the application" — nothing in `main` or `cli`
distinguishes exit reasons and nothing needs to. Asking systemd instead would need
`org.freedesktop.systemd1.manage-units`, which the unprivileged `karaoke` account does not get, where
exiting needs no privilege at all.

**The availability predicate is `INVOCATION_ID`.** systemd sets it in every unit's environment and a
`cargo run` never has one. It answers *"I am supervised"*, which is both what a restart requires and
what distinguishes the appliance from a developer's Linux desktop — where logind *would* honour a
poweroff, and must not be asked. The reasoning is in
[`Power is a capability of the host, not a method on the machine`](../decisions/api-and-network.md#power-is-a-capability-of-the-host-not-a-method-on-the-machine).

**`TimeoutStopSec=30s`, and what is in the budget.** The block is bounded everywhere but its tail:
`persist()` is a single settings write and runs first, the API stop is an explicit 3 s, and the wait
for the poll thread is an explicit 5 s.

**One poll interval is not what that wait costs**, which is why it is a `recv_timeout` on a channel
the thread sends down as its last act rather than a `JoinHandle::join`. `Machine::poll` reaches
`advance`, so a pass can be the next song's file being opened and its decoder built — seconds, for a
video on a spinning disk — and `join` has no timed form to bound it with. Its answer decides one
thing: `Machine::clear_auditions` runs only when the thread has finished, because clearing while it
still runs is what lets a retry be armed behind the clear. A wait that expires therefore skips the
clear and says so, and `Machine::new` purges at the next start exactly as it does after a kill.

**`persist()` moved in front of both waits** for the reason the save exists: it is what the owner set
from their phone, and nothing about it depends on either thread having stopped.

`drop(runtime)` is **unbounded** — dropping a multi-thread runtime waits for
`spawn_blocking` tasks, and `ops::off_runtime` carries package installs, so a restart during one can
genuinely sit there. Unset, the default 90 s means a press that leaves the television lit for a
minute and a half, which reads as a button that did not work and is the thing somebody then holds
down.

**Measured on the box, 2026-09-08, and one line of it is a trap.** A single press produced exactly
what it should:

```
15:21:33  systemd-logind[688]: Power key pressed short.
15:21:33  systemd-logind[688]: Powering off...
15:21:33  systemd[1]: Stopping karaokemachine.service - KaraokeMachine...
15:21:34  systemd[1]: karaokemachine.service: Deactivated successfully.
```

**But `journalctl -t karaokemachine -b -1` does not end with `shutting down`** — the machine's own
last lines are simply absent, because journald is being shut down alongside everything else and the
final writes never reach the disk. Somebody checking the obvious way will conclude the machine was
killed where it stood, which is the *original* bug this whole area is about, and be wrong.

**`Deactivated successfully` is the line that actually answers it**, and it is in `-u`, not `-t`. It
is what systemd logs for a main process that exited by itself with status 0; a SIGKILL at the
timeout logs `state 'stop-sigterm' timed out. Killing.` and `Failed with result 'timeout'` instead.
Since the shutdown block is the only path to a clean exit 0 — a panic is non-zero and a kill is
logged — that line means `persist()` ran. The stop took **one second** against the 30 s budget.

So the two commands to check a press by are:

```sh
sudo journalctl -b -1 -u karaokemachine | tail   # Deactivated successfully = clean
sudo journalctl -b -1 | grep -i 'power key'      # logind saw it at all
```

and `-t karaokemachine` — normally the *only* useful one here, per *Three things a real screen
found* — is the misleading one for this single question.

**Also measured: nothing grabs the power button.** The box has two, `event3` (`PNP0C0C`) and `event4`
(`LNXPWRBN`), and SDL holding either would have starved logind of the key. `Power key pressed short.`
with the machine running is the proof that it does not.

**The start limit is left at its default and is worth knowing about.** No `StartLimitIntervalSec` or
`StartLimitBurst` is set, so five starts in ten seconds puts the unit in `failed` and it does not come
back. With `RestartSec=2` and a startup that reads a catalog and takes a screen, a person pressing a
button cannot reach that; a wedged restart loop could. The defaults are a real crash-loop net and the
recovery is `systemctl reset-failed karaokemachine`.

**Four things can stop a power press working, and they are told apart by asking rather than
reasoning.** The deploy installs a `logind.conf.d` drop-in pinning `HandlePowerKey=poweroff` and
warns when `acpid` is enabled; the rest is diagnosis on the box:

```sh
busctl get-property org.freedesktop.login1 /org/freedesktop/login1 \
  org.freedesktop.login1.Manager HandlePowerKey   # what logind believes now
loginctl list-inhibitors                          # anything holding handle-power-key
systemctl is-enabled acpid; ls /etc/acpi/events/  # a second handler
grep -B2 -A5 -i 'power button' /proc/bus/input/devices   # which eventN it is
sudo evtest /dev/input/eventN                     # refuses if something has EVIOCGRAB'd it
```

The last is the one that cannot be reasoned about from here: SDL's evdev path opens every
`/dev/input/event*` the session's uaccess ACLs expose, and whether it grabs a device tagged
`power-switch` is a question for the box. **Test with the machine stopped and again with it
running** — working stopped and not running is the signature, and nothing else produces it.


## The boot race, twice

**First: the appliance booted headless after every power cut.** The unit's `After=` waited for a seat
and a sound card and **nothing for the graphics device**. The *kernel* finishes with i915 nearly a
second before **udev** creates the device node, and the application is in between — three consecutive
boots came up with no screen, reporting `active (running)` and answering the API the whole time.

**`After=dev-dri-card0.device` is the declaration this wants to be, and it does not work.** udev tags
block, net, tty and sound devices for systemd but not drm, so that unit is permanently inactive and
ordering against it means the service never starts at all.

**Second: the same thing again, one layer up.** Both guards passed and it still failed, because
neither was the predicate. SDL's `get_driindex` requires a connector that is `DRM_MODE_CONNECTED`
**and has at least one mode**, and i915 creates the node about a second before it finishes probing
that. So the first fix passed on **margin** rather than because waiting for the node was right — the
same 300 ms was enough one day and not the next, which is what a margin is.

The script waits for the thing itself now: `/sys/class/drm/card*-*/status` reading `connected` with a
non-empty `modes`. Three things learned the hard way:

- **`[ -s ]` cannot be used on `modes`.** Every sysfs attribute stats as one page whatever it holds,
  so `-s` is true for the empty file this exists to wait past. It has to be read.
- **The glob is `card*-*`, not `card0-*`**, so a box with an integrated and a discrete GPU finds the
  connector that is actually lit.
- **One other failure produces a character-identical log line** — `open` returning `EACCES`, i.e.
  udev's `uaccess` ACL not yet applied — which is what the old flat `sleep 0.1` was guessing at. The
  script asks, and reports which precondition it waited on.

**It is a script, not an inline `ExecStartPre=/bin/sh -c`**, for a reason that cost two boots: systemd
expands `$WORD` as one of *its* variables before any shell sees it, so a loop counter became empty and
the bound collapsed to something that errors. The trap is that **`systemctl show` prints the stored
argv with the dollars still in it**, so the line reads as correct in precisely the place you would go
to check it.

**The real lesson is that no `ExecStartPre` could have been the whole fix.** People switch the
television off at the wall, which drops HDMI hotplug detect, so on such a boot the connector reads
`disconnected`, the script correctly times out, and the information that somebody switched the set on
arrives minutes later. `Restart=` is no help either: a missing display is non-fatal, so **the process
never exits and systemd has nothing to act on**. A unit reporting `active (running)` for ever is
precisely the shape of this whole family of faults.

So `display::run` returns a typed error and the caller keeps asking — every two seconds for the first
thirty, every fifteen thereafter. **The typing is the careful part**: `DisplayError::Unavailable` is
produced by exactly one line, `sdl.video()`, and everything after it lives in a function whose
signature cannot return it. So a `?` added to that body later cannot quietly become retryable — a
missing font or a renderer that will not create still falls back to headless, because those do not
mend themselves and retrying them is a loop that never ends.

`wait-for-drm.sh` has a test pointing environment variables at a fabricated sysfs tree. **A shell
script with a test is unusual here and is earned**: it decides whether the appliance comes up with a
picture, it runs on one machine in the world, and the state that broke it — `connected` with an empty
`modes` — is trivial to fabricate and was impossible to catch any other way. Every case asserts exit
status 0, which is the invariant that must never break.

## Everything before systemd

The product decision is `What the box shows before the machine does` in
[`distribution.md`](../decisions/distribution.md). This is how it is built, and what building it
found.

`tools/platform/linux/appliance-boot.sh [user@]host`, or `task deploy:linux:boot HOST=user@box`, is
the once-per-box act: it installs Plymouth, hides the bootloader menu behind a one-second any-key
window, and selects the theme the `.deb` already put on the box. `--revert` undoes both from backups
it takes exactly once. It is the only
piece of this tooling that asks for a password, and that is a property of a correctly locked-down
box rather than a shortcoming — `update-grub`, `update-initramfs` and `plymouth-set-default-theme`
are not things an unattended deploy should be able to do.

**A Plymouth splash and kmsdrm want the same thing, and only one can have it.** Plymouth is a DRM
client: while the splash is up it holds DRM master, which is precisely what SDL's kmsdrm backend
calls `drmSetMaster` for. Both `plymouth-quit-wait.service` and the machine's unit are ordered
`After=systemd-user-sessions.service` and nothing else relates them, so left alone the ordering is
undefined — and the failure when the machine loses is the family this whole document is about: a
black television from a unit reporting `active (running)`, because a display that will not start is
non-fatal.

**The obvious fix is `After=plymouth-quit-wait.service` on the machine, and it is the wrong way
round.** It was here for one afternoon and it works, in the sense that the machine never fights for
the device — but it makes the splash leave *first* and the machine load afterwards, which on this box
is four seconds of black television between a mark disappearing and a picture arriving.

The ordering runs the other way now. The machine's unit is `Type=notify` and sends `READY=1` once its
audio, catalog and API are up and one line before it starts the display; a drop-in orders
`plymouth-quit.service` and `plymouth-quit-wait.service` after it. So the splash covers the load and
goes at the last possible moment, and the machine's own unit names Plymouth nowhere at all — which is
why a box without it is unaffected without a word being said about it.

**Ready means everything except the screen**, and it cannot mean anything else: the display cannot
start until Plymouth releases the device, and Plymouth will not release it until READY arrives. That
is not a fudge — `display::run` is called in a retry loop precisely because a television is a thing
somebody switches on hours later, so "ready" was never going to be able to include a picture.

**What happens if READY never comes** is the risk this buys, and it is bounded: `TimeoutStartSec=45s`
fails the unit, `Restart=always` tries again, and everything ordered after it runs anyway, because
`After=` is satisfied by a failure as much as by a success. Plymouth is released either way rather
than holding the screen for ever. Forty-five seconds rather than the default ninety, because what is
being bounded is how long a television can show a boot splash and nothing else.

`sd_notify` is fifteen lines in `notify.rs` rather than a dependency on `libsystemd`: the protocol is
one datagram to one socket named by `NOTIFY_SOCKET`, and the alternative is a C library on the build
of every platform that has no systemd. The abstract-socket form (`@`-prefixed) is reported and
skipped rather than silently ignored, because a silent no-op there is a black television with nothing
anywhere saying why.

**`-R` is the load-bearing letter.** `plymouth-set-default-theme` without it writes the choice into
`/etc` and stops; the theme has to be in the *initramfs*, which is the only thing read before the
root filesystem is mounted. The symptom of forgetting it is a boot that is now silent and shows
nothing, which reads as the theme being broken rather than absent.

**`GRUB_TIMEOUT=0` is unavailable under EFI, and nothing says so out loud.** `keystatus` — the
held-SHIFT test that lets a BIOS box show its menu with no timeout — is a BIOS facility. Under EFI
it simply is not there, so a hidden menu with a zero timeout has no way in at all. The decision
argues the number; the difference is invisible in
`/etc/default/grub`, identical on both firmwares, and only discoverable by knowing which one the box
has. `appliance-boot.sh` prints the firmware for that reason.

**The `/etc/default/grub` edit is a filter, and it has a test.** The third shell script here to earn
one, and by the widest margin: `wait-for-drm.sh` decides whether the appliance comes up with a
picture, and `grub-appliance-edit.sh` decides whether it comes up. Making it `stdin → stdout` rather
than an in-place editor is what buys the test — a dozen starting files, no root, no bootloader, no
box — and it leaves the backup and the write with the caller, which is the half that has to be
careful rather than the half that has to be correct.

Three properties are worth naming, because each is a bug that would only ever be found on hardware:

- **Idempotence.** The caller cannot know whether a box has been through this before, and a filter
  that appends a little more each run is how a kernel command line comes to read `quiet quiet quiet`.
- **A `key=value` word already present is replaced where it stands**, not appended beside itself.
  Two `loglevel=` words happen to work, because the last wins — which makes it a bug that tests
  fine and reads as a mistake to everybody who sees it afterwards.
- **Commented lines stay commented.** Every distribution's shipped file carries commented-out
  examples of the exact keys this sets, and uncommenting one is the script answering a question
  nobody asked it.

**Two guards before anything is written**, both of which need root to evaluate and so cannot be the
operator's job. The theme directory has to be there, or the `.deb` is not installed and selecting an
absent theme gives a boot with no splash at all. And `grub.cfg` must not contain `osprober` entries,
because hiding the menu on a dual-boot box takes the other operating system away from anybody who
does not know to press a key. `osprober` is the precise test rather than counting `menuentry` lines:
a plain single-OS Debian has a top-level entry for itself and usually one for the firmware setup, so
a count says two and refuses for no reason.

**`nomodeset` is warned about and not removed.** With it there is no kernel mode setting, so there
is no splash *and* no picture from the machine either — but taking a word off somebody's kernel
command line is a larger liberty than anything else here takes.

### What a television showed that no check here could

The first boot with all of it in place, watched from the sofa: three seconds of black, **two seconds
of the set saying it had no HDMI signal**, the mark for about a tenth of a second, five seconds of
console text, then the machine. Every automated check had passed. Two of the four are fixed and one
of them is the reason the GRUB window is now one second rather than three.

- **The two seconds of no signal are the panel re-syncing at the i915 handover**, and nothing here
  fixes them. The firmware lights the screen, GRUB inherits that mode, and then i915 loads and
  re-drives the connector — which on HDMI means dropping the link and negotiating again. It happened
  before this change too; hiding the menu only removed the thing that was on screen during it.
  `GRUB_GFXPAYLOAD_LINUX=keep` narrows the window and does not close it. **The honest lever is the
  timeout**: whatever the hidden GRUB window is set to is dead time stacked directly on top of it,
  which is what took the window from three seconds to one.
- **A tenth of a second of mark, then five seconds of text**, and both halves were one mistake. The
  theme's `SetQuitFunction` blanked the mark — written on the reasoning that leaving it up would
  fight the machine's first frame — and Debian's `plymouth quit` then restored the text console into
  the five seconds the machine spends loading its SoundFont and catalog. `[  OK  ] Finished
  plymouth-quit-wait.service` where the picture should be reads as a crash, not as a boot.

The fix is three lines in three files and none of them work alone: the quit function holds the mark
at full opacity, a drop-in makes `plymouth-quit.service` pass `--retain-splash` so the frame survives
the daemon, and `systemd.show_status=false` stops the console drawing over the frame that survived.

**The drop-in goes in `/etc` and not in the package**, because it edits *Debian's* unit — the same
line the disabled service and the unselected theme are drawn on: the package ships things, the
script decides them.

**And the frame it retains was being wiped a fifth of a second later**, by `TTYReset` and
`TTYVHangup` in the machine's own unit. Both reset the virtual terminal, resetting a VT repaints the
console, and the television finished negotiating HDMI into the black that left behind. They are gone;
the section on the seat above says what they were for and why nothing is lost.

### Where the twenty-three seconds go

`systemd-analyze`, on the appliance, is the measurement that reframes all of the above — before this
work, and after it:

```
6.055s (firmware) + 6.750s (loader) + 5.856s (kernel) + 4.823s (userspace) = 23.486s
6.034s (firmware) + 4.926s (loader) + 6.586s (kernel) + 4.939s (userspace) = 22.487s
```

To a *first frame*, which is the number that matters and which `systemd-analyze` does not report, it
went from about 27.6 s to about 23.9 s. **The firmware is now the largest single item and nothing
here can touch it**: six seconds of POST before the bootloader is a setting in the box's own setup
screen, and it is worth more than everything below.

**The loader is the surprise, and the initramfs is why**: 6.75 seconds reading a 12 MB kernel and a
**72 MB** initrd off a SATA disk — about 12 MB/s, which is GRUB's own disk path rather than the
disk's fault. `MODULES=most` is Debian's default and builds an initramfs for hardware this box does
not have; `MODULES=dep` builds one for the hardware it has.

**It is a flag rather than the default, and the reason is where the cost falls.** Everything else
`appliance-boot.sh` does is an appliance decision and reversible over ssh. This one is the owner's:
an initramfs built for the hardware present will not boot hardware that changes, so a swapped
storage controller or a disk moved to another machine needs a rescue USB. `--slim-initramfs` writes
`MODULES=dep` into `/etc/initramfs-tools/conf.d/` — conf.d rather than `initramfs.conf` for the same
reason the loader's text is hidden from `/etc/grub.d` rather than by editing `10_linux` — and prints
the initramfs size before and after, so the saving is a figure off the box rather than a claim in a
comment.

**Measured: 71.9 MB down to 23.7 MB, and the loader from 6.757 s to 4.926 s.** That is 1.83 s, where
dividing the sizes by the observed 12 MB/s predicted nearer 4.5 s — so a good part of the loader's
time is fixed overhead rather than bytes, and an estimate from a read rate over-promises here. The
saving is real and it is less than half what the arithmetic suggested, which is the reason the script
prints both sizes rather than a percentage.

**GRUB's own text outlives all of it.** `10_linux` prints `Loading Linux …` and `Loading initial
ramdisk …`, and `quiet` says nothing about the bootloader — so those two lines own the screen through
the loader *and* the kernel phase, until i915 modesets. It is the longest-lived thing on the
television during a boot, and it was the one artefact left after the splash was fixed.

`10_linux` guards the echoes with `quiet_boot`, which Debian hardcodes to `0` four lines from the top
— **and setting it there is the obvious fix and the wrong one**, because `10_linux` is a dpkg
conffile and an edited copy earns a conffile prompt at every `grub-common` upgrade, on a box with no
keyboard. `/etc/grub.d` is a documented extension point instead (its own README hands the number
namespace between 10 and 20 to the administrator), so `appliance-boot.sh` drops a `09_` script there
that sets `color_normal=black/black`. The text is still printed and cannot be seen.

**What that costs is written into the file it generates**: GRUB's interactive command line and the
hint under the menu use `color_normal` and go invisible with it. The menu *entries* use
`menu_color_normal`, which `05_debian_theme` sets separately, so the realistic recovery — press a
key, choose an older kernel — is untouched.

**The machine's own first frame is about 4.1s after its service starts**, measured cold, and the
rows below are what the splash now spans rather than a list of things to shorten:

| | | on screen |
|---|---|---|
| `wait-for-drm.sh`, waiting for a connected connector | 0.59s | the mark |
| the SoundFont | **1.76s** | the mark |
| the catalog and the packages folder | 0.31s | the mark |
| SDL, to a first frame | 1.35s | **black** |

**The split matters much less than it did, and only the last row is still visible as a gap.** READY
goes out at the end of the third row, so the splash covers everything above it and the remaining
black is SDL initialising — about a second and a half, and the last thing anybody sees before the
machine's own screen.

**The SoundFont is the only large reducible item and it is not the machine's to reduce**: the bank on
that box is 274 MB where the shipped one is 32 MB, and 1.76s is a cold read of it off a SATA disk. On
a warm restart the whole startup is 1.35s, because the page cache has it. Deferring the load was
considered and dropped: it would move the first frame earlier by shortening a stretch the mark
already covers, and would do nothing at all for the one row that is black.

Recorded for whoever does reach for it: `lib.rs` puts the API bind ahead of the display deliberately,
so that the connect panel has a URL on the first frame — and on this box's cold boot **there is no
URL at that point either.** The machine logs `no reachable address` at bind and the address arrives
thirty seconds later from the connect refresher. The ordering buys nothing on a cold boot; it buys
something on a warm restart.

**`--retain-splash` does not retain anything on this box, and the flag is kept anyway.** The screen
goes black the instant `plymouthd` exits, whatever it was asked to leave behind, because the
kernel's own console takes the framebuffer back and repaints it. **The wrong explanation is worth
recording because it survived two rounds**: it looked as though something reclaimed the framebuffer
*after a while*, so shortening the gap would let the retained frame through. It was measured across
a four-second gap and again across 1.45 s and the length made no difference — the repaint is
immediate. What actually fixed the picture was making the gap short, not making the frame survive
it, and those two are easy to mistake for each other because they produce the same improvement.

Removing `TTYReset` and `TTYVHangup` belongs to the same correction. They *were* wiping the screen
and removing them was right, but on their own they bought nothing visible, because the console
repaints regardless.

### Three things the first real run found, and two of them were in series

**`command -v` asked a different shell than the one that would run the command.** Every tool this
needs lives in `/usr/sbin`, and a non-root Debian login has
`PATH=/usr/local/bin:/usr/bin:/bin:/usr/games` — no sbin on it at all. `sudo` finds them regardless,
because sudoers carries its own `secure_path`. So `command -v update-grub` answered *no* about a
command `sudo update-grub` would have run perfectly, and the same probe silently reported the boot
theme as `?` on a box where it was correctly set. The script looks for the *file* now, across the
sbin directories, which is the question that matches what happens next.

**And the report that should have caught it was `regenerate 2>&1 | sed 's/^/  /'`.** A pipeline's
status is its last command's, so `sed` succeeding buried the failure and `set -e` had nothing to act
on: the run wrote `/etc/default/grub`, failed to rebuild `grub.cfg`, printed one line saying so, and
went on to a state block that looked entirely healthy. **The two were in series** — the first made
the step fail, the second made the failure invisible — which is the same shape as the two
silent-staleness faults above, and the reason nothing here is indented through a pipe any more.

Editing the defaults file without regenerating is worth naming as its own error rather than a
warning: `grub.cfg` is what boots, and `/etc/default/grub` is only ever an input to producing it. So
a box with a defaults file and no regenerator now **stops before writing anything**, and the
`--revert` path deliberately sits *above* that check — a box that cannot rebuild its config is
exactly the box whose owner most needs to put it back.

**Debian's `plymouth-set-default-theme` does not parse a theme, it greps one.**

```sh
MODULE_NAME=$(grep "ModuleName *= *" …/$THEME.plymouth | sed 's/ModuleName *= *//')
```

Over the whole file, comments included. This repository writes a comment block at the top of every
file it ships, and that block opened by naming the key it was about to set — so the grep returned
two lines, they landed inside `[ ! -e … ]`, and the script failed with `[: too many arguments`
naming its own line number and nothing else. **A convention that is right everywhere else in the
tree is wrong in a file somebody else reads with `grep`.** The prose is reworded, and
`plymouth_theme.rs` asserts each key is assigned exactly once *using grep's own matching rules* —
anywhere in a line, not anchored at its start, because a test anchored at the start passes on the
exact file that failed.

## The machine advertised nothing after every cold boot

The same bug in a different crate: something sampled once, during a boot in which nothing was ready,
and never sampled again. `bind` called `Advert::start` three seconds before there was an address. The
connect refresher recovered the **URL list** twenty-six seconds later — so `/discover` reported the
right address — and never looked at the advertisement. The machine answered HTTP perfectly while
announcing nothing, and a remote that finds a machine only by mDNS could not see it.

**`After=network-online.target` would not have fixed it**, and it is worth knowing why before anybody
reaches for it: that target was reached nearly four seconds *before* dhcpcd acquired carrier — with
ifupdown and dhcpcd it is satisfied vacuously. And it would only ever have covered boot; a lease that
moves the machine at nine in the evening is the same fault with no target to hang it on.

`run_advertiser` owns the advertisement start to finish and `bind` does not open one. Two owners
would be two error paths and a window in which both hold an `Advert` for the same name, and nothing is
lost by waiting because `interval` fires its first tick immediately. It is a **separate task** from the
connect refresher — a slow address enumeration and a slow service registration should not queue behind
one another, and working out an address and announcing one are two jobs.

Two orderings that are easy to get wrong:

- **The old `Advert` is dropped before the new one is made.** `Drop` unregisters *by fullname*, and a
  replacement carries the same fullname — so registering first sends a goodbye for the record just
  published, and every phone browsing the network drops the machine it has only just found.
- **Shutdown awaits the aborted task before taking the slot.** `abort()` asks rather than waits, so an
  `Advert` sitting in a local is dropped whenever the runtime next gets to the task — possibly after
  the server has returned, by which point there is no runtime left to send the goodbye on.

The policy is a pure function tested directly, including that reordering the *tail* is not a change,
since the ranking can flip without anything becoming unreachable. **The first entry is the exception,
and it became one when the advert started publishing it.** It goes out as the `url` TXT record — the
address the machine has chosen, which a browsing client cannot work out for itself because the
ranking that produces it reads interface names and an A record carries none. So a reorder that moves
a different address to the front changes a published fact rather than only an ordering, and
`advert_action` compares the head in place and the rest as a set. Left as a pure set comparison it
would strand a stale URL in the advert until something else happened to change the set. See
`The advert names the address the machine chose` in `docs/decisions/api-and-network.md`.

## Audio on the box

**The ALSA card order moves between boots, and following `default` is not enough.** The USB CODEC and
the HDA Intel have swapped numbering, and `default` follows card 0 — so every sample went to one
interface while the headphones were in the other. `/proc/asound/…/status` reading `RUNNING` while the
room is silent is what that looks like, and it is worth knowing as a diagnostic: **the audio path can
be perfectly healthy and still be pointed somewhere nobody is listening.** That is what
`audio.output_device` and the Linux preference for a USB interface exist for.

**Heard on the box**, with the CODEC the only PCM in `RUNNING` at 48 kHz stereo while every HDA
substream stays closed — the USB preference working on real hardware, where it had only ever been
exercised against a fixture and against WASAPI.

**The preference is re-applied at every start, and nothing is written into `settings.json`.**
Recording what it resolved to on the first start is the failure it exists to prevent wearing the
opposite hat: a machine told to prefer USB ends up naming one PCM, so the next reorder moves the
sound exactly as before. The worse case leaves no trace — boot once with the CODEC unplugged and
`"system"` is recorded, which reads as
*the owner chose to follow the system* and disables the preference permanently. On a box with no
keyboard the symptom is silence and a healthy-looking service, which is the same symptom as the
original fault. So: `audio.output_device` on this box should be **absent** unless somebody chose,
and an absent key is the correct state to find rather than a machine that has not finished setting
itself up.

**What is proven and what is not.** Proven: the machine makes sound, out of the chosen interface.
Still not proven: the rest of the room — the interface into the mixer, the mixer into the extractor,
the extractor into the receiver. Headphones tap the output *before* any of that.

One measurement worth keeping: on the direct `hw:` device the stream negotiated a **5 ms period**, a
quarter of what the stock `default` gives through `dmix`, and held it with no xrun. That is the clock
the display rides, so the "the display is only as smooth as the audio period" defect is comfortably
answered on this path.
