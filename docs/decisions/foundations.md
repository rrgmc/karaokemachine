# Foundations

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Language

**Rust** — `midly` parses karaoke meta-events completely, and `rustysynth` is a pure-Rust SoundFont
synthesizer. So the whole audio path has no C dependency and no GC in the render callback. Go was
rejected for GC in real-time audio and cgo-only GUIs; C++ for its dependency-management cost.

## Graphics

**SDL3** — the right answer for a full-screen, GPU-accelerated karaoke display, and the best Android
support of the candidates. Vendored and statically linked via `sdl3-rs`.

## Front end scope

**SDL3 shows the singer display and song-number entry only.** All searching, queueing and settings
happen through the API.

**One exception**: a `.kmpkg` dragged onto the window installs. It is not a settings surface and
opens no new capability. It is a fourth way to hand the machine a path that three other routes
already take. It belongs on the window because a window is the one thing a file can be dropped onto.

## SoundFont

**Bundle a compact General MIDI bank** so it works out of the box, with a setting to point at a larger
`.sf2`. The bank is **GeneralUser GS v2.0.3** (31 MiB), under its License v2.0: use without
restriction including commercial, modification and repackaging allowed. Not committed —
`tools/setup/fetch-assets.sh` fetches it, pinned by commit and checksum. FluidR3_GM and
**MuseScore_General v0.2** (206 MiB, MIT) are the documented overrides.

**An override bank is never installed into `assets/`**, because everything there is copied into every
release. `tools/setup/fetch-assets.sh --bank <name>` caches it and installs it nowhere, and
`Switching the SoundFont locally` is how one is then played. `assets/soundfont/` therefore holds
exactly one bank, and `tools/dist/check-assets.sh` refuses a staging run where it holds two.

**The checkout overlay is unreachable from a staged or installed machine.** Dropping a bank into the
tree works for `cargo run` and does nothing at all for the build somebody is about to listen to.

Measured in [`docs/architecture/audio.md`](../architecture/audio.md).

## Minimum Android version

**API 26 (Android 8.0, 2017).** `libaaudio.so` does not exist below it and the audio backend links
against it. Building for API 21 fails at the link with `unable to find library -laaudio`.

## What the product is called

**`KaraokeMachine`, one word, wherever a person reads it except under an icon.** The three siblings
are `KaraokeMachine Package Builder`, `KaraokeMachine Remote` and `KaraokeMachine Admin`, abbreviated
to `KM …` **on an icon and nowhere else**. A common word between four names is not enough to say they
are one family. A common *prefix* is, and it is the half that survives being read.

**The lowercase spelling is a different name.** `karaokemachine` is what this is called to cargo, to a
shell, to a package manager and to `directories`. So the binaries, the data directories, the `.deb`,
the systemd unit, the bundle ids, the Android `applicationId` and every clap `about` use it.

**Nor is the common noun the name.** *The karaoke machine is not answering* would be as true of
anybody else's box, so it stays lowercase. The test is whether the sentence *means* the trademark
when the phrase is replaced with it. An install preset reading *Just the karaoke machine*, sitting
directly above a component list, is the trademark.

**Two places measure the name rather than assuming a width**, because fourteen characters do not fit
everywhere seven do. The attract screen drops to the smaller face when the name will not fit. That is
a real branch on a phone in portrait and on nothing else. The television banner sets the two halves
touching on one line at a fitted size. It keeps the two-tone mark while the words read as the single
word they are.

**A phone launcher gives nine characters, so it gets a label rather than the name.** It cuts
`KaraokeMachine Remote` inside the first word, and `KaraokeMac…` names neither the family nor the
program. So **the remote's icon says `KM Remote`** on both phones. That says which family *and* which
program in the space available. The amber and green separate two icons side by side as a second
signal rather than the only one.

**An icon gets the abbreviation and everything else gets the name, and the line is drawn at what
sits in a list of siblings.** Short covers the macOS bundle and the `.app` folder, the Linux
`.desktop` entry and the Windows Start menu shortcut. It also covers every installer component, the
Android activity label and the iOS `CFBundleDisplayName`. A Start menu, a Dock and a component list read the three siblings
beside each other and beside the machine. Four entries all beginning `KaraokeMachine ` differ only
past the width the list will give them.

**Long: everything read one at a time** — the window title bar, the macOS application menu and the
tray tooltip. **"The desktop" is not the test; width pressure is, and only a list has any.** A title
bar is one program's. A menu bar item is read where the name is the *subject* rather than a way of
telling two things apart. A tooltip is hovered deliberately. None of the three is competing for
width with a sibling.

**And long inside the program, always.** This covers a page's `<title>` and its header, the admin
pages' program label, and the sentence at the top of an exported backup. Anywhere a program is
already open and talking about itself, abbreviating is a saving nobody asked for.

**A page and an icon therefore read two constants and never one.** One string serving both is the
shape that puts the icon's nine characters into a page header. A shortening applied to the launcher
travels down the constant into every `<h1>` the program draws, and nothing can disagree with
anything.

**The machine keeps the whole name everywhere**, because it is the product the other three are named
after. It is also the one that appears alone: under a television, with no sibling beside it to be
told apart from. It has no abbreviated spelling at all, not even on its icon.

**Under an icon the machine's name is two words, `Karaoke Machine`.** A launcher that cannot fit
fourteen characters on one line cuts or wraps a single word wherever the width runs out.
`KaraokeMa` over `chine` reads as neither word, and a space gives the launcher the one break that
reads. The surfaces are the Android activity label, the iOS `CFBundleDisplayName` and
`CFBundleName`, and the macOS `Karaoke Machine.app` with both of its plist fields. They also include
the Linux `.desktop` `Name`, the Windows Start menu folder, its shortcut and the desktop shortcut. Everything else keeps
the one word, including the Android application label, the window title, `AppName`,
`FileDescription` and every page.

**The streaming launcher's icon says `KM Stream`.** `Karaoke Machine Stream` reads as a fifth product
rather than a way of starting the first, and it is the longest label in the list. `KM …` is the icon
form the siblings already use, so the launcher sits in the list as one of them. That means
`KM Stream.app` on macOS and the `KM Stream` Start menu shortcut on Windows. The Linux stream action
is an item on the machine's own menu rather than an icon of its own, so it keeps
`Stream to a television`.

**Android's names are the ones to copy.** `ports/remote/android/…/values/strings.xml` carries
`app_name` = `KaraokeMachine Remote` beside `app_name_short` = `KM Remote`, wired to the application
and activity labels. So the launcher says the short one, while Settings, the task switcher and the
local-network permission sentence say the whole name. That is this rule on the platform that states
it in two files. It is why the Rust constants are `APP_NAME` and `APP_NAME_SHORT` rather than a
fresh vocabulary.

**The two platforms are not symmetrical about this, and copying one to the other does not work.**
Android has two knobs: a launcher reads the *activity's* label and falls back to the
*application's*, so the long name survives beside the short one. Apple has one:
`CFBundleDisplayName` is the home-screen label. `CFBundleName` is the shorter fallback of the pair,
not a longer alternative. So an iOS bundle has nowhere for the full name to sit. There it survives in the local-network permission sentence, which is read with room, and nowhere
else.

**macOS's desktop bundles have the same one knob and it costs nothing there**, so the three
`Info.*.plist` files say `KM …` in both fields. What a plist governs is the Dock, the Finder and the
task switcher — exactly the list. The running program sets the window title and the application
menu, so the whole name reaches them from a Rust constant instead.

## What the tool calls itself

**Three audiences and three spellings: the page names the product, the icon names it in nine
characters, and the command line names the command.** Every page of the package builder reads
`APP_NAME`, which is *KaraokeMachine Package Builder*. So do the window title, the macOS application
menu and the tray tooltip. `APP_NAME_SHORT` beside it is *KM Package Builder*. It goes on the Linux
`.desktop` entry and on the two sentences naming `KM Package Builder.app`, a folder on disk.

**Two constants and not one, however much the same words in one place reads like an economy.** The
page's name and the icon's are two decisions, and one string holding both makes a shortening of
either a shortening of both. Two can be checked against each other; one cannot be checked at all.

The startup banner, `--help` and the "could not reach…" toast keep `km-package-builder`. There the
crate name is the thing somebody typed and the thing they would grep for. That is not an
inconsistency. A program with a Dock icon and a document type should not introduce itself by its
cargo package. And a shell should not be told a product name it cannot type.

**`km-admin` has its own `APP_NAME` in `tools/cmd/assets/km-admin/src/server.rs`** — its pages, its
window title and its tray tooltip say *KaraokeMachine Admin*. Its `--help`, its startup banner and
the console line beside them say `km-admin`. **It needs no short twin.** It writes no `.desktop`
entry, so every place its icon is named is a file that cannot read a Rust constant. Its *tab* names
are not its own, though — see
[`Two admin surfaces, one vocabulary`](distribution.md#two-admin-surfaces-one-vocabulary).

**Four files spell a name out because none of them can read a Rust constant.** All four say the
*short* one: `tools/platform/macos/Info.package-builder.plist`, `Info.remote.plist`,
`Info.admin.plist`, and `bundle_name()` in `tools/dist/cmd.sh`. The Windows installer's shortcut and
component names are a fifth copy for the same reason.

**The four Windows build scripts are the sixth**, and they say the *full* name. They set
`FileDescription` in `crates/machine/karaokemachine/build.rs`, `tools/cmd/km-package-builder/build.rs`,
`crates/remote/km-remote/build.rs` and `tools/cmd/assets/km-admin/build.rs`. Windows Firewall puts
that field in front of somebody deciding whether to let a program onto their network. So it is a
name and not the crate's `cargo` sentence. A build script runs before its crate compiles and cannot
read `APP_NAME` out of it.

Those are the only ones and they are named here so a rename knows where to go.

`km-remote` carries `TITLE` — *KaraokeMachine Remote* — rather than an `APP_NAME`. It has one window
and no page of its own to head, and all three things it feeds are read one at a time.
`km-remote-pages` needs no constant at all: its pages have no product header, and its `<title>` is
the single place it says the name.

## Unsafe code, once

**`unsafe` is denied workspace-wide, and every exception is narrowed to one item and made to say what
it is for.** `crates/platform/km-console/src/lib.rs` holds the only whole-*module* allowance —
`#![allow(unsafe_code)]`, and it is two calls. Everywhere else an exception is a
`#[expect(unsafe_code, reason = "…")]` on a single item, and there are thirty-nine:

- Twenty-seven are in the machine and the shells around it. They cover the C symbols Android's
  loader, SDLActivity and Swift look for, and the liblog bridge. They also cover two borrowed C
  strings SDL does not wrap, and the texture-destruction order SDL demands.
- Two are in `km-display`, for that same texture rule.
- Nine are in `km-audio`, where two of the three mixers are reached as C. Every call on WASAPI's
  `IAudioEndpointVolume` is an `unsafe fn`, and `CoInitializeEx` has to be paired on the thread that
  made it. CoreAudio's AudioObject properties are four functions taking raw pointers. The ALSA half
  of that module needs none, `alsa` wrapping its own C.

**The thirty-ninth is in `km-stream`, and it is a binding's mistake rather than a C API.** A muxer
flagged `AVFMT_NOFILE` opens every file it produces itself, and `AVFormatContext.pb` is specified to
stay null for one. But `ffmpeg_next::format::output_as` opens a file regardless, where
`output_to_stream` beside it tests the flag and refuses. So the HLS muxer is handed a playlist this
process is holding open. On Windows its rename onto that name fails with nothing reported. The
segments are right and the temporary playlist beside them is right, but every client fetches an empty
one.

`avio_closep` on that field is the whole of the exception. The obligation is small enough to check by
reading it: nothing has written a header yet, so nothing is reading `pb`. And null is the state the
muxer is specified to be given.

**The lint level stays `deny` rather than `allow`**, so every exception has to name its reason in
order to compile.

**`km-console` is a crate rather than a module because three programs need the same answer.**
`karaokemachine.exe`, `km-package-builder` and `km-remote` are each GUI-subsystem on Windows. Each
prints from an early-exit flag path, and a copy in each would be **three `unsafe` exceptions**
instead of one.

**`open_url` deliberately does not live here.** Folding a process launcher in would widen the crate
from "is anybody reading?" to "and also start processes". So `km-remote` carries its own copy of that
twenty-line function instead.

**Nothing in the crate knows which program it is in, deliberately.** Whether anybody is reading is a
property of how a process was launched, not of what it does. Windows *gives* a console to a
console-subsystem executable that was double-clicked: a black window beside the application. So the
process asks `GetConsoleProcessList` whether anything else is attached, and it calls `FreeConsole`
when nothing is. One process attached means nobody typed this; two or more means a shell did. Neither
call has a safe wrapper anywhere.

**`println!` panics when stdout has gone**, and the banner is printed at eleven places. So freeing the
console would abort the process on every double-click and never once when run from a shell.
Everything that prints goes through one `say`, which prints when there is somewhere to print and logs
when there is not.

Deciding whether a *handle* can be written is `AsRawHandle`, which is safe.

## Nothing about startup is sampled once

**The machine keeps looking for the things that were not there when it started.** Two faults on one
cold boot of the appliance said this was a rule and not a bug fix. The television was black, and the
machine was invisible to the offline remote. In both cases something had been sampled once, during a
boot in which nothing was ready yet, and never sampled again. So a television switched on at nine in
the evening lights up by itself. And a machine whose DHCP lease arrives three seconds after the API
binds becomes findable without anybody restarting anything.

**The wrong fix for each is the same shape** — reaching for a stronger `After=` in the systemd unit.
That cannot work for either. `network-online.target` was reached nearly four seconds before the box
had carrier, and no `ExecStartPre` can wait for a television that is switched on later.

**`Restart=` cannot rescue it either.** The machine treats a missing display and a missing network as
non-fatal, because a box with no screen still has an API and a queue. So the process never exits,
systemd has nothing to act on, and the unit reports `active (running)` throughout. Everything says it
is fine.

**What is deliberately *not* retried is anything that will not mend itself**: a missing font, a
renderer that will not create, SDL refusing to start. Those fall back to headless and say so once.
Retrying them is a loop that never ends and a log that never stops. The boundary is a type rather
than a message, so it cannot rot. Exactly one line produces `DisplayError::Unavailable`, and
everything past it lives in a function whose signature cannot return it.

**The cost is one connector probe every fifteen seconds**, less often than the kernel already polls
the same connector, and one address comparison every five.

## No removable media

**Nothing in this product models removable media.** There is no mount detection, no eject and no
"volume absent" state. Nothing distinguishes *a file that is gone* from *a file on a disk that is not
plugged in right now*. A folder that is not there is a folder that is not there.

**A library may still live on an external drive or a stick**, named in `settings.package_dirs`. That
is a supported arrangement, and `package_dirs` was built for it. What is refused is the machine
*reasoning* about the difference. No code asks whether a missing folder means the owner removed
something or merely unplugged something. On a karaoke machine under a television, no answer to that
question is worth the state it would cost.

**Two decisions lean on this row.** `Where packages live` in [`packaging.md`](packaging.md) resolves
packages by folder rather than by a list of file paths, and `The catalog is what the folders hold`
reconciles the catalog against a scan. Neither is sound without it. So adding removable-media support
reopens both, and this row has to move first.

**A "the drive is out" state was declined on cost.** Every place that reads a folder would need to
distinguish three answers instead of two. The owner would need somewhere to see and dismiss the
state. The reward is politeness about a case a home appliance meets rarely, and recovers from by
itself the moment the folder is back.

## Only `debug.` names a file

**Outside the `debug.` namespace, `settings.json` never persists the path of an individual content
file. A folder, yes. A file, no.** And its positive half, which matters as much: **`debug.<kind>` is
where naming a file is legitimate** — one list per kind of file-based data. Each holds extra files
layered *on top of* whatever the folder rules resolved, never replacing them.

**Why a folder is a different kind of thing from a file.** A folder is a place, and places are stable:
what is in it can change without the setting becoming wrong. A file path is an assertion that one
particular file will be at one particular spelling for ever. An ordinary tidy-up falsifies it. A
rename inside the same folder is enough, and what it produces is a message above the title that
nothing can clear.

**`debug.` earns the exemption rather than being handed it.** `debug.soundfonts[].path` and
`debug.play_file_roots` make the namespace the place paths live. `debug.soundfonts` is already the
*additive* shape: slot 1 is the bank the machine resolved for itself and slots 2–9 are extras layered
over it.

**The rule is about persisting a path as an instruction, not about paths in the program**: the catalog
stores each installed package's absolute path. That is a **record of what was scanned** rather than a
statement of where to look.

**What `debug.` names belongs to the owner, not to the machine.** Nothing the machine does deletes a
file a `debug.` entry points at. No API route writes to that section. Only a person at a keyboard
changes it. An owner may empty a list whenever they like, and `--clear-debug-soundfonts` removes
entries rather than files.

**`display.font` is out of scope rather than an exception.** This row governs file-based *data* — the
content an owner accumulates: songs, pictures, banks. A font is a resource the build ships. A missing
one degrades gracefully by falling through to the bundled face and then a candidate list. So it
exhibits none of the fault this row exists to remove.

## One spelling per concept, across every surface

**A concept has one name: the same name in Rust, in SQL, on the wire, in a URL and on a command
line.** Where two surfaces disagree, the one that matches the underlying function or column
wins; where neither does, the majority of the surfaces does.

Five HTTP surfaces, three dozen cargo aliases, sixty tasks and eleven command-line programs each grow
consistent with themselves and inconsistent read across. None of the spellings is wrong from inside
the surface using it.

**Two things the rule does not say.** It does not make a *shorter* name wrong: `q` is the search box
on all three surfaces and stays. And it is about *which name* a concept gets, not how a word is
spelled. A surface that disagrees with itself about spelling is
[`The prose and the names are US English`](#the-prose-and-the-names-are-us-english)'s business rather
than this row's.

**`Language::code()` and `Locale::tag()` are different words on purpose.** Song tags want the word
for a third thing, and of the three only two are called what their standard calls them. A BCP 47 tag
really is a tag, and a word somebody types onto a song really is one. The ISO 639-1 value is a code,
and every other surface says so. `Language`'s private field and the 184 table rows behind it are
spelled `code` too. A `self.0.tag` inside `code()` would be the same collision one layer down.

Reading them apart is where the rule's cost shows. A bare `.tag()` grep over the workspace returns 43
sites, and eight of them are the `Locale` one. So the reading is the work rather than the edit.

Renames here carry no alias and no shim; see `No compatibility aliases` in [`songs.md`](songs.md).

## The prose and the names are US English

**One dialect, and it reaches everything: prose, Rust identifiers, file names, SQL columns, wire
fields and command lines.** `color`, `catalog`, `license`, `favorite`, `normalize`, `gray`, `center`,
`analyze`, `artifact`.

**What the rule does not reach:**

- **A third party's field names.** `km-wallpaper-pack` deserializes `license` from Openverse, Pexels and
  Pixabay, and those keys are theirs to spell.
- **SPDX identifiers, and `LICENSE-APACHE` / `LICENSE-MIT`** — the licenses' own names.
- **A translated catalog.** `en.ftl` is source prose and this rule governs it. `pt-BR.ftl` is a
  translation of that prose, and Portuguese governs it. `pt-BR` as a directory name is data, the
  same as the `en-GB` below.
- **Data that is not prose.** `en-GB` in `km-kmpkg`'s language tests asserts that a region-qualified
  tag is *rejected*, and `ENGL` / `PORT` / `ITAL` are corpus strings a normalizer reads. Both look
  like spellings and are neither.
- **A deliberate misspelling.** `km-wallpaper-pack`'s `a_mistyped_key_is_refused` feeds a credits file
  the key `licence_url` precisely because it is wrong, and `deny_unknown_fields` has to refuse it.
  That is also the likeliest typo anybody will now make. That makes the test better and its comment
  load-bearing.

**A stem rule is safe only where every inflection keeps the stem.** `centre` → `center` is right for
`centres` and `epicentre`. It is wrong for `centred`, which becomes `centerd`, and that lands in
comments and CSS where no test or formatter looks. The `-re` → `-er` family is exactly where the `e`
moves, so spell those inflections out rather than trusting a stem.

**Where a rename meets data somebody has, it is a numbered migration step.** An
`ALTER TABLE RENAME COLUMN` arm of the version ladder renames a `.kmbuild` column, by the rule in
[`A store opens at its current version or is refused`](#a-store-opens-at-its-current-version-or-is-refused).

**The one line that could fail silently is in Swift.** `Server.swift` names the catalog mirror and its
`-wal`/`-shm` siblings in a literal array to exclude them from iCloud backup. The quota is 25 MB per
app, and the mirror is well past it. A rename confined to Rust would leave that array naming a file
that is not there, with nothing to say so. Sweep the whole tree at once rather than crate by crate.

## A store opens at its current version or is refused

**A store holding somebody's work opens at a version this build knows, or it is refused with both
numbers in the message.** An open that read an older shape as the current one would go on to write
over it. A curation database holds months of hand work that nothing can rebuild. A refusal that names
what it found and what this build opens is the answer that loses nothing.

**The number keeps counting.** A change to a store's shape is the next number, reached by one numbered
step from the number before it. Raising the oldest version a build opens retires the steps every
store in service has passed. The count never starts again: a store stamped 14 must never meet a build
to which 14 means a different shape.

**A store with no number is refused as well, except a new one.** Each store says what *new* looks
like, because a new file carries no number either.

**A newer store is refused too, for the same reason.** SQLite hands back rows from a table with
columns a build has never heard of. So a build that opened a newer database silently would curate it
while ignoring whatever the newer build added.

| Store | Its number | What a new one looks like |
|---|---|---|
| `.kmbuild`, the curation database | `PRAGMA user_version`, from `OLDEST_SCHEMA_VERSION` to `SCHEMA_VERSION` | no `songs` table yet, and it is stamped current |
| `.kmbackup.json`, a curation backup | `format` | always written with the current one |
| `settings.json`, the machine's settings | `settings_version`, `CURRENT_SETTINGS_VERSION` | no `settings_version` at all, which is what an installer writes |

**A backup is the one store read past a newer number.** A newer backup is read as far as this build
understands it, and the difference is reported. A backup is what somebody has when the database is
gone. An older one is refused like any other store.

**A refused `settings.json` does not stop the machine.** It is set aside as `settings.json.bad`, the
same path an unreadable file takes, and the machine starts from defaults. A box under a television
that will not come up is the worse failure by a distance. And the file is still there for whoever
wants what was in it. A build only ever writes its own shape. So every file this build writes is
stamped with the current version, whatever the value in memory says.

**A store with no number is judged on its shape**: the columns and tables every query reads. The
offline remote's `favorites.sqlite` is somebody's work. In an older shape it is refused and left
exactly as it is. Its `catalog.sqlite` is a copy of what the machine still holds. In an older shape it
is **dropped and fetched again** instead, because a derived store costs a download to rebuild and
never somebody's work. See
[`The mirror is thrown away when its shape moves; the favorites are refused`](remotes.md#the-mirror-is-thrown-away-when-its-shape-moves-the-favorites-are-refused).

**The machine's `library.sqlite` is derived the same way**, from the packages it reinstalls at every
start. So an older shape drops its song tables, and the next start fills them again. It keeps `meta`
through the drop, so the catalog version goes on counting up and every mirror is told to fetch. A
restarted counter could land on the number a mirror already holds and read as *already up to date*.

## The interface has a locale; a song has a language

**Two concepts, two words, and they are independent.** A `language` is what a song is sung in. It is an
ISO 639-1 code in a package, `?language=pt` on a route and a section in the printed book. A `locale` is
what a *surface* speaks: `en` or `pt-BR`, chosen by whoever is reading. A machine set to `pt-BR`
still has an English section in its song book. And `GET /songs/book.pdf?language=pt&locale=en` is a
reasonable thing to ask for.

`locale` is a new word rather than a second meaning for one that was taken. It is spelled the same
everywhere: `machine.locale` in `settings.json`, `km_locale::Locale`, the `km_locale` cookie,
`?locale=` on the book route, `i18n/pt-BR.ftl`.

**English is the source and every other catalog is a translation of it.** A key is written in
`en.ftl` first; a key only a translation has is a leftover, and a test says so in both directions.

**Latin-1 is the ceiling for a *locale*.** A song's words and a song's title are not bound by it:
the television draws CJK, per `Non-Latin text` in [`songs.md`](songs.md#non-latin-text). What a
locale is bound by is the interface's own furniture. `km_display::words::is_drawable` holds every
shipped catalog to Latin-1 and a named handful. Whichever font the platform supplied draws those
strings, and nothing stands behind it for them. The printed book is a second ceiling of the same
height, speaking cp1252 with no embedded font.

**A Japanese *locale* is a larger job than a Japanese *song*.** The interface would have to open a
CJK face before it can draw its own first screen. That is the cost `Non-Latin text` defers until a
song asks. Portuguese needs no glyph Latin-1 lacks, so a second locale needs no font work.

**A language names itself in a picker.** `Português (Brasil)`, not `Portuguese (Brazil)`: the one
person who has to read a language picker is by definition the one who cannot read the page around it.
That is the only string in the product deliberately not in the reader's language. It lives in code
rather than in a catalog, so that translating it is not possible.

**What a machine says and what a page says are set separately**, because they answer to different
people. A television is in a room and the room has one language, so it is a setting. A phone belongs
to one person, so a page follows their browser and remembers their choice. Two people at one party
read one queue in two languages.

## What a user reads is written in plain application language

**Every surface a person uses speaks the plain, conventional English of a software application.**
Short labels, ordinary sentences, standard terminology. Three readers, and the register is theirs
rather than the writer's:

| Reader | Surfaces | Register |
|---|---|---|
| a singer at a party | the remote, the television screen | labels; at most one short sentence |
| an operator setting something up | `km-admin`, the package builder, `/admin/`, `--help`, the console, a release page | a sentence or two, consequence first |
| a maintainer reading the source | code comments, `docs/` | the reasoning, at whatever length it takes |

**A page is not a comment.** Where the reasoning behind a control is worth keeping, it belongs in the
`{# #}` or `//` beside the markup. It does not belong on screen, where it costs a reader who came to
press a button.

**Out, on any surface in the first two rows**: aphorism, inverted sentences and rhetorical contrast
of the *X is not Y, it is Z* shape. Em-dash asides are out too, and so is any sentence whose subject
is the design rather than the thing the reader is doing.

**Plain does not mean shorter.** A fact a reader acts on survives the rewrite. That `km-admin` never
*searches* for songs is the commonest wrong expectation about it, and it stays as a plain sentence.
`--lan` still says it raises a Windows firewall prompt once per program, port and network profile.
What goes is the argument around the fact.

**The rule also heads each Fluent catalog.** Each catalog names its own reader:

- The singer's remote says its words are *read on a phone, at a party, by somebody holding a
  microphone*.
- `km-admin-pages` says its reader is deciding something and may be given a consequence.
- `km-admin`'s says its reader is an owner setting a machine up on a desktop.
- The package builder's says its reader is a curator working a corpus of hundreds of thousands of
  files.

It is written here as well because clap-generated `--help` has no catalog to head. Structurally,
`--help` is doc comments on a `Cli` struct.

**A translation answers to its own language.** A `pt-BR` string is written to be plain
Portuguese rather than a word-for-word rendering of the plain English — see
[`The prose and the names are US English`](#the-prose-and-the-names-are-us-english).
