# Persistence

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

`directories` for platform config and data directories: `settings.json`, `library.sqlite`, a
`packages/` folder and a `soundfonts/` folder. **No list of registered package paths** — what a
folder holds is what is installed.

`auditions/` sits beside them and is the one that is **scratch rather than storage** — a song
uploaded through `debug/play-upload` is staged there so it can be played as an ordinary loose file.
Nothing in it outlives the **song** that played it. A staged audition is removed as soon as it stops
being the loaded song — `Machine::reclaim_auditions`, called from the two places `state.loaded` is
written — because it is never catalogd and can never be played again. A folder that will not go is
looked at again eight times at 250 ms, since `VideoSong`/`CdgSong` hold the reader on the machine's
side and the display thread's per-frame `Arc` outlives the displacement by one drawn frame; Windows
will not delete an open file. Two folders exist only between staging an upload and starting it: one
song playing, one still arriving over the wire. **The peak is therefore what is playing plus one
upload in flight**, each capped by the route's gigabyte limit — the number a television is checking.
The purge at startup and the sweep on the way in remain as backstops for a run that was killed.
Resolved by rule like `packages/`, so no setting names it — the setting that exists,
`debug.enabled`, answers *whether* not *where*.

## Three rules about the settings file

**Nothing in `settings.json` is required** — every field has a default, unknown keys are ignored, and
a file that will not parse is moved aside as `settings.json.bad` while the machine starts from
defaults. **A karaoke machine that refuses to boot because a JSON key is missing is a broken
appliance.** Writes go to a temp file and are renamed, so an interrupted write cannot leave a
half-parsed file.

**A *new key* needs no migration and no `settings_version` bump**, and the distinction is what that
field exists for: `settings_version` moves when a field's *meaning* changes under a file already on
disk, and the change comes with one repair step keyed on the new number, while `#[serde(default)]`
fills a new key on every file ever written. A file naming a version below the one this build writes
is set aside as `settings.json.bad` and the machine starts from defaults — see
[`A store opens at its current version or is refused`](../decisions/foundations.md#a-store-opens-at-its-current-version-or-is-refused).
A file naming none is current, because that is what an installer writes.

**A key that is *removed* needs a migration.** This is the third rule, and the first two do not
cover it: removing a field is neither a new key nor a changed default. Removing one is
*mechanically* safe —
unknown keys are ignored, so an old file still parses and the key simply stops being written — and
that silence is the trap: whatever the owner had typed there is dropped on the first save with
nothing said. So a field being retired keeps a **private, deserialize-only** twin
(`#[serde(default, rename = "<the old key>", skip_serializing)]`) for exactly as long as the
migration needs to read it, and the migration decides where the value goes. Private, so that "nothing reads
this any more" is a compiler error rather than a grep.

And whatever the migration decides, it must not decide it by *renaming* the key. A rename would make
every existing value invisible to the very migration meant to convert it, because an unknown key is
ignored rather than refused — so the key keeps its spelling and the migration reads it through the
twin.

**A fourth rule: a new repair step gets a new version, even when the previous one landed the same
day.** A machine that started between two changes sharing a number is stamped by the first and skips
the second for ever, and nothing shows it: a fresh install is already current, and a test that builds
its fixture at the previous version exercises the gate that does work. Reusing a version number that
has been stamped on any real install makes the gate a lie.

**One key is read twice, and by two different readers.** `logging` decides how much this run says
and where it says it, which has to be settled *before* the subscriber exists — and `Settings::load`
writes, so reading it through `load` would create a `settings.json` for a machine that has never run
and undo the ordering in `cli::main`. So `km_logsettings::peek` parses the file into a struct holding
that one section and nothing else, never writing and never complaining: a file that will not parse
peeks as no opinion at all, and `load` is a moment behind it to report the fault properly and do the
rename. **The same peek serves the package builder and `km-admin`**, which each keep a settings file
of their own and each face the same ordering; the section and its grammar are one crate so that three
readers cannot become three dialects. `machine_locale` is the same shape for the same reason, for `--song-book`. **Two readers of
one file is a cost worth naming**, and the thing that keeps it cheap is that neither of them decides
anything the other does — a peek answers one question and `load` owns the rest.

**When settings are written.** Most changes wait for shutdown: a singer nudging a mic slider should
not cause a disk write, and losing the last adjustment to a power cut costs nothing. Installing or
removing a package is different and is written **immediately** — it is a deliberate, durable action,
and losing it would mean the catalog silently shrinking on the next start — which is what
hard-killing the process just after an install would otherwise produce.

## One machine per data directory

**`machine.lock` in the data directory, held open for the life of the run.** `Claim::take` is the
first thing `run` does, before anything reads or writes the directory, so a second machine over one
machine's state is stopped while it has still done nothing rather than after it has opened the
catalog and started answering for a queue the first one owns.

**The conflict is the directory and not the port.** `The machine holds its port, rather than claiming
it once` makes a failed bind a state the machine is built to sit in and recover from — a television
switched off at the wall, a DHCP lease that arrived late — so a bind failure cannot mean *something
else is running* without contradicting the decision that exists because a machine which gave up is
one nobody notices. What genuinely cannot be shared is one settings file written whole, one packages
folder and one catalog.

**One per directory is a better rule than one at a time.** Two machines with two `--data-dir`s on
two ports are two machines and they work, which is exactly what the development port table assumes.

**Nothing has to be cleaned up.** The operating system releases the lock when the file closes, on
exit *or* on a crash, so there is no stale claim to reason about — a machine that was killed leaves
a file behind and no claim on it. `std::fs::File::try_lock` is what does it, which is why this costs
no dependency and no `unsafe`; `try_lock` and never `lock`, because waiting would make a second
machine hang with nothing said when the answer is a different directory.

## The packages folder

`<data_dir>/packages`, created empty on purpose — **the asset directory deliberately is not**, because
an empty one there disguises "nothing installed", whereas this is the place an owner is *meant* to
find.

**`packages_to_install` sorts by file name, and that sort now carries the whole stability guarantee
on its own.** Order decides which of two packages wanting one bank keeps it and which takes the next,
so an unsorted scan would give the same folder different numbers on two starts — a printed book going
stale for no reason anybody could see. Nothing else holds it up. Paths go through `tidy`, because the
spelling the catalog stored need not match the one a directory walk produces.

**A package there that will not open is logged, skipped and reported above the title**: that is a
fault to fix, and it should keep saying so at every pass until somebody does.

**Deduplication moved out of the scan and became a question about ids.** `machine::startup_plan` is
the deciding half now: it takes the candidates — `debug.packages` first, then each scanned folder —
and drops any whose **package id** it has already offered this pass. By id rather than by path
because the same package can be present twice under two names, in two folders or as one of the `-2`
copies the drop route mints, and `Library::install` replaces by id: installing it twice is *correct*
and merely re-indexes every song in it for nothing, at every start, without saying so. Paths are
deduplicated first as well, because it is free and it catches one file named twice.

**What it keeps is the first by name, which is why the file's name is the manifest's to choose.** The
plan walks the sorted candidates in order, so a folder holding two files of one package serves
whichever sorts first — as likely as not the older build. `dropped::place` names an incoming package
`PackageMeta::file_stem` and sweeps any other file of the same id out of the write folder, so the
pair only arises from a file somebody copied in by hand. See
`What an installed package file is called` in `docs/decisions/packaging.md`.

### Settings name folders, and uninstall deletes the file

**`settings.package_dirs` is the only way to say where packages are.** Naming individual *files* is
what makes such a list fragile: each entry is a standing obligation, so a file renamed or moved
inside its own folder becomes a failure the owner has to clear by hand. A folder is stable, and a
package taken out of one simply stops being installed. `debug.packages` is what remains for naming a
single file, and it sits behind `debug.` precisely because that fragility is what it is for.

**Uninstall deletes the file, and there is no ignore list.** An ignore list exists only because a
folder scanned at every start would otherwise put back what the owner had just removed, and a file
that has been deleted cannot come back. **An API call that destroys the owner's only copy of a
package is sharp**, and that is answered rather than dismissed: `packages.uninstall` ships **admin**,
the deletion is logged at `warn` because it is the only account anywhere of a destroyed file, and a
package reached through `debug.packages` is refused outright, because a file the owner keeps
somewhere of their own is not the machine's to remove.

## Which settings reach the API

`km_api`'s settings are **the knobs that belong to a performance** and that a phone in somebody's hand
should be able to turn. How long the box keeps its sound card is set once when the machine is
installed, so it is a settings key and not an API field.

**`audio.output_device` is the exception, and the exception is instructive.** It is installation
configuration by exactly the same test, and it *is* on the API — because the test it fails is a
different one: an operator has to be able to **discover** the identifiers before they can name one,
and on an appliance there is no screen to discover them on. So it gets **routes of its own** rather
than a field in the settings DTO, which is what keeps the rule intact: the settings route still
carries only performance knobs, and nothing about the output device rides the 250 ms state broadcast.
`--list-audio-devices` covers the case where the API is not reachable yet at all.

**The output's own level rides beside it, on those routes and not in the DTO**, for the same reason
and one more: it is not the machine's to store at all. The operating system already remembers an
endpoint's level, so a copy in `settings.json` would overwrite at every start whatever had been set
from anywhere else. The machine reads it when asked and writes it when told, and keeps nothing.

**The debugging switch is the second such exception, and it followed the pattern rather than
inventing one.** Installation configuration again, on the API again, and for a recognizably similar
reason: the machines where it is most wanted are the ones with no way to edit a file. It gets a
route of its own, `PUT /api/v1/admin/debug`, and not a `SettingsPatchDto` field — so the rule above is
still intact and still says the same thing. It *does* have a `GET` twin, `GET /api/v1/debug`, and
`/discover` carries the same bit as `debug_enabled` — a client has to ask before it sends a
gigabyte, which is exactly the half a switch needs. Note that `--list-audio-devices`' counterpart does not exist here:
there is no CLI flag for uploads, because the two platforms the switch was added for — Android and
the appliance — are the two with no command line to type it on.

**`demo.enabled` is the third, and it is the first one where the split is in the request rather than
between two routes.** `demo` is a settings section — `enabled`, `delay_secs`, `min_suitability` —
and two of the three now reach the API, each through a route of its own and neither through
`SettingsPatchDto`, exactly as the two above — `delay_secs` is the sixth entry below. What is new is that the same route can write the
setting or not: `{"enabled": true}` moves a run-only flag on the machine and leaves the file alone,
and `persist: true` writes it as well. So the machine holds **two** values — `State::demo_enabled`
for the run and `settings.demo.enabled` for the next start — and `GET /api/v1/demo` reports both, as
`enabled` and `stored`. Collapsing them would make the un-persisted switch indistinguishable from
the persisted one to every client.

A persisted change is written immediately rather than at shutdown, joining the package install in
the paragraph above and for a recognizably similar reason: it is a deliberate, durable act, and
losing it to a power cut means the machine quietly doing something other than what it was told.

**`machine.name` is the fourth, and it is the plainest.** Installation configuration, on the API,
`PUT /api/v1/admin/machine/name` rather than a `SettingsPatchDto` field — the rule holding for the fourth
time without needing a new argument. It has no `GET` twin for the debugging switch's reason,
`/discover` having carried the name since long before anything could set it. It *does* have a CLI
flag where uploads has none, because the machine has a command line on the two desktops where a
`.kmpkg` is built and named, and `--set-name` is what a person reaches for before the machine has a
password of their own -- the generated PIN is on the screen, but a command line is quicker for
somebody already at one. Persisted immediately, joining the package install, the session-epoch
write and a persisted demo switch above.

**And it is the first of the four with a live counterpart that has to be told.** The name is
published on the network as well as answered over HTTP, so `ApiState` holds the running value and
the mDNS advertiser compares it every five seconds. Writing only the file would have renamed the
machine everywhere except the place a phone looks.

**`machine.locale` is the fifth, and it is the same shape with the opposite live story.** Installation
configuration, persisted immediately, `POST /admin/machine/locale` on the page and
`PUT /api/v1/admin/machine/locale` on the API — but nothing has to be told,
because the display reads `Machine::locale()` per frame rather than holding a copy. That is a lock and
a `Copy` enum sixty times a second, deliberately not `settings()`, which clones everything the machine
remembers; the picker therefore takes effect on the next frame rather than the next start.

It needs no migration: every field is optional and the default is English, so an existing file reads
as it always did. A tag with no catalog falls back to English rather than refusing to start — the
worse failure by a distance is an appliance under a television that will not come up.

**`demo.delay_secs` is the sixth, and it is where the *nothing to discover* test stops being the whole
of the question.** The argument for keeping it off the API is that a plain number a settings file
states perfectly well has nothing to enumerate the way `audio.output_device` does. True, and beside
the point on a television box: the machines where the
delay is most wanted are the ones with no way to edit a file, which is the same fact that put uploads
and the demo switch on routes. So it gets `PUT /api/v1/admin/demo/delay` — a route of its own rather
than a field on `PUT /api/v1/admin/demo`, because that route's body is *for tonight or for good* and a
delay has no run-only half: it is **always** written down, immediately, joining the four above.
Refused with a 400 above `km_api::machine::MAX_DEMO_DELAY_SECS`, an hour, which bounds the route and
not the file.

**And it is the first with a live counterpart inside the machine itself.** `demo_resume_at` is already
armed as *last deliberate act + delay*, so a write here shifts it by the difference rather than
re-arming it from now — `karaokemachine::machine::demo_deadline_moved`. Writing only the settings
would leave a machine obeying the old delay until the next time somebody touched it, which on an idle
box is never.

`min_suitability` is the one key of the three off the API, for a reason of its own: it is the demo
setting a person cannot judge from the room.
A delay is a length somebody feels; a floor of 5 against 6 is a claim about what the packager measured
about a file, and choosing it wants the catalog in front of you.

## A notice that ended in a bare colon

Three places take the *file's name* out of a package problem and deliberately refuse to show its path,
and each says so in a comment. All three then printed a reason with the whole path in it, **twice**:
the error type names the file in every variant it has, and the installer wrapped a second
`could not open {path}:` around that.

**On an appliance the redundancy compounded into an *empty* error rather than a verbose one.** The
notice gets two lines above the title, the wrapper breaks only on whitespace so a Windows path is one
unbreakable word, and the cut was silent — line one held the prefix, line two the first copy of the
path, and the line carrying the actual reason was thrown away with nothing to mark it. What reached
the television read as a machine that will not say what is wrong.

Both wrappers are gone and the cut is now marked with an ellipsis — **that silence is what hid the
evidence**. Stripping the prefix rather than rendering the error a second way is the one judgment
call: both strings come from the same path inside one call, so the prefix being removed is one this
code put there — whereas a path-free set of error strings plus an accessor would ripple into `km-pack`,
whose CLI output genuinely wants the file named.
