# The interface

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Windowed mode

**Desktop only.** On Windows, macOS and Linux `F` toggles fullscreen and `Esc` leaves it (or quits,
from a window), with the cursor shown only while windowed. **Android is always fullscreen** — it is a
fixed-function appliance with no desktop to return to, and a remote that dropped the television out
of fullscreen would leave nobody a way back. Compiled out there rather than merely unreachable.

**Where it starts is decided by who put the machine there, not by a default.** A machine a setup
program installed starts fullscreen; one built out of a checkout, or unzipped from a portable folder,
starts in a window. `DisplaySettings::default()` is reached by an install with no settings file, and
the other thing with no settings file is a checkout somebody has just built — so one default serving
both takes the whole screen away from the person debugging it. The default is off and the three setup
programs pre-write `{"display": {"fullscreen": true}}` where there is no settings file, which is
[`The setup programs pre-write a settings file`](distribution.md#the-setup-programs-pre-write-a-settings-file).

**How the window was left is written back when it closes**, so `F` outlives the process. It was a
control somebody could work and could not keep: the toggle moved the window and nothing else, and
every restart went back to what `display.fullscreen` had always said. Somebody who takes a machine
fullscreen and closes it has said what they want that machine to do.

The write is skipped where the answer has not changed, so an ordinary close touches nothing — `save`
writes the whole file, and a run that rewrote it on every exit is a run that can lose an unrelated
hand edit to a power cut at the wrong moment. Android is excluded outright: fullscreen there is the
platform rather than a preference, and writing it back would let a fact about a television follow a
data directory onto a desktop.

**`--fullscreen` and `--windowed` move one run and write nothing down**, the discipline `--api-bind`
already keeps — **and that is now what decides whether the close writes anything at all.** The two
rules are one rule: a run that was told what to do does not get to speak for the machine, so a
debugging run against an appliance's own `--data-dir` cannot turn its television into a window from
then on. A flag run that is *then* toggled with `F` still writes nothing; the whole run was declared
temporary at the point it was started, and that is a smaller surprise than the alternative.

The pair rather than one flag, because either direction is something somebody has to be able to
override: a debugging run against an appliance's settings, and a checkout build driving a real
television. `F` still toggles from wherever a flag left it, and neither reaches Android, which has no
command line.

**No migration carries a default to machines already installed**, deliberately. A default only ever
reaches an install with no settings file.

## The window can be told to stay in front

**`T` holds the window above every other application, and `display.always_on_top` is where it is
remembered.** Beside `F`, because the two are the same question about the same window: how much of
the screen the machine takes, and whether it keeps it.

**It is for the machine that shares a screen.** One driving a television owns the panel and will
never reach for this. One on a desk beside a browser and a chat window is behind them on the first
click, and the words go with it — which is the whole of what a karaoke machine is for. Fullscreen is
not the same answer and cannot be: the point here is that the other windows stay usable.

**A letter and not a `Ctrl+F<n>`.** The modified function keys are the debugging shelf —
`Ctrl+1`…`Ctrl+9` for banks, `Ctrl+F12` for the strip — and the two product controls among them,
`Ctrl+F10` and `Ctrl+F11`, are there only because each is carried by the bare key it pairs with.
This is a control an owner is meant to find, and the letters are for whoever is standing at the
machine.

**The window is asked rather than tracked**, exactly as fullscreen is, because a compositor may
decline. Wayland is the case that does: a client there does not get to place itself in the stack. A
key that does nothing on that desktop is the honest outcome, and better than a machine that refuses
to start on it.

**Written back when the display closes**, on `Windowed mode`'s rules and one short of them. The
write is skipped where nothing changed, and Android is excluded — a platform showing one application
at a time has no stack, so an answer written there would follow a data directory onto a desktop that
does. What is missing is the flag half: there is no `--always-on-top`, so no run is declared
temporary and every run may speak for the machine. A flag pair exists for fullscreen because a
debugging run against an appliance's `--data-dir` must not turn its television into a window; an
appliance has no window to raise, so the case that argument protects cannot arise here.

**No setup program pre-writes it.** An installed machine is the television, which is the one case
that does not want it.

## The window opens where it was left

**A machine closed in a window opens at the same size and in the same place.** `display.width`,
`display.height`, `display.x` and `display.y` hold the rect, and the display writes them back when
it closes. A machine on a desk is moved beside the other windows and sized to fit between them, and
losing that at every start is losing the arrangement somebody made.

**Only a close in a window writes it.** A machine closed fullscreen keeps the rect it had, which is
also the one `F` returns to. A maximized or minimized window writes nothing either: neither is a
rect somebody chose, and a maximized rect saved as a plain one can no longer be restored to anything
smaller. `F` goes back to the rect the window left, not to the middle of the screen.

**Any run writes it, including one given `--fullscreen` or `--windowed`.** Those flags decide how
the window starts, and [`Windowed mode`](#windowed-mode) keeps a flag run from rewriting that. Where
a window sits is a different question, and a debugging run against an appliance's `--data-dir`
changes nothing on its television by moving a window the television never shows. The write is
skipped where nothing changed, on `Windowed mode`'s reasoning, and Android is excluded because it
has no windows.

**A saved position that lands on no connected display is ignored, and the window opens centered.**
The test is whether the title bar, the part a window is dragged by, is on a screen. A monitor
unplugged since the last close is the case: the position was real when it was written, and opened
on a desk with one screen fewer it is a window nobody can see or reach.

**A position is a pair or nothing.** A file holding only `x` centers the window rather than
guessing the other half. No setup program pre-writes any of the four keys.

## Application icon

**`KM` on a near-black plate, over three angular bands of color — generated rather than drawn.** One
design serves every platform: Windows, macOS, Linux, Android and the two web pages. Three layers:
bands fill the rounded tile, a near-black plate sits over them, and the monogram stands on the plate —
the **K** in the theme's near-white, the **M** in the hue that names the program.

**The M is colored because the wordmark colors it**, which is why the icon is letters rather than a
picture: `site/index.html` sets the name as `Karaoke<span>Machine</span>` and the stylesheet gives
that span the amber. `KM` on all of them rather than per-program initials, because these are several
programs and **one product** — `PB` and `RM` are initials nobody has ever seen written down.

**16 pixels is where the monogram is weakest, and that trade was taken with eyes open.** Two letters
in a twelve-pixel plate are a smear, so at that size the palette carries the identity rather than the
mark.

**The bands are straight-edged, and the ground has a floor rather than a ceiling.** Angular rather
than radial: a tile made of a radial gradient is a surface and nothing else. The mark stands on the
plate and clears it by 15:1, so what constrains the bands is a floor — `icon_ground` neat is **1.2:1**
against the plate, which is a missing edge rather than a dark one, and is lifted partway toward
`icon_glow` at about 2.2:1. `crates/playback/km-display/src/icon.rs` holds both ends: **4.5:1** for
each letter against the plate, **1.8:1** for the plate against every band. Two numbers because a
letter has to be *read* and a plate edge only has to be *seen*.

**Rendered from signed distance fields by `km-display`'s `icon` example**, for the reason the
wallpapers and the TV banner are generated: the artwork is defined once, in the display's own `Theme`,
so it cannot drift from the app or between platforms. Every stroke is a capsule, and the letters are
cut to a cap-height band so terminals come out flat while joins inside a letter keep their round caps
— that intersection is why there is no rotation anywhere in the renderer. **No full wordmark inside
the icon**: two initials are not the name, and the name appears only on the Android TV banner, the one
tile a launcher draws no label for.

**One drawing, four palettes** — the machine leads with amber, the package builder with the theme's
blue accent, the offline remote with the second accent green, and `km-admin` with the magenta. Each
is an existing `Theme` color rather than one invented for the icon, the same discipline as taking the
amber from `lyric_sung`.

**The hue reaches exactly two places: the widest band, and the M.** The deep purple, the magenta, the
plate and the K are byte-identical across all four, and a test pins that. A shared ground would make
the icons *identical*, since the K is the same near-white in all of them; a second *purple* per
program would be a second design arriving without a decision.

The reason each program needs its own is that they are run beside each other. Two taskbar buttons and
two Explorer entries wearing an identical icon cannot be told apart, and the macOS installer puts
three bundles in one `/Applications` folder. Green is the furthest hue from the other two at 16
pixels.

**`km-admin` is where "every color comes from `Theme`, and not one is new" costs something.** There
is no unclaimed hue left: `alert` means *warning*, `icon_ground` is too dark to lead, and the two
`lyric_*` colors are the words on a screen. It takes `icon_glow` — already the bright end of the
supporting bands in every mark — so its lead band and its middle band are the same hue differing in
value, and the tile reads as three steps with one a fold. **The collision is accepted rather than
designed away**, because tinting its supporting bands is what turns four programs into four designs.

**The magenta is lifted 12% toward white to be a letter.** Straight, the M measures **4.00:1** against
the plate, under the floor. The number was bisected: 0.04 still fails at 4.23:1, 0.08 is the first
that clears, and 0.12 is used because a value sitting exactly on a floor is one rounding change from
failing. It is applied where the palette is *read*, never in `Theme` — lifting it there would change
the bands of all four marks to fix one letter.

**Loose sizes follow one rule: every file has a reader.** The machine has seven, the builder six, and
the remote two — the 32 its own page serves and the 256 the icon test samples — because the remote
installs nothing into `hicolor` and declares no document type. All three have an `.icns` written from
the same `ICNS_MEMBERS` list, which carries its own sizes and so moves no loose-size count. The
remote's Android launcher entry writes `mipmap-*/` from the same drawing in the same green, with no
banner, because a banner is a tile a television's home row draws and a remote never reaches one.

**The favicon is picked by the shell, not by the page**: `km-remote-pages` carries both marks, so the
remote the machine serves at `/` stays amber and only the standalone one turns green.

**There is no fifth hue.** A fifth program needs either a new `Theme` color — reversing the rule this
row is about — or a different axis entirely, and that is a decision to make deliberately rather than
discover.

## A badge says how the machine was started

**The streaming launcher wears the machine's mark with a broadcast badge on it**, and that badge is
the second axis the row above says a fifth mark has to come from. A hue names a *program*; a badge
names a *way of starting one*. The four hues are spent, and spending a fifth here would say the
streaming machine is a fifth program when it is the same executable given `--stream`.

**It takes the machine's own amber and adds no color at all**, so the rule that the hue reaches the
widest band and the M survives intact. The badge is a source and two waves in the strip of plate the
letters leave empty below their baseline — the glyph every platform already draws for this, in the
orientation they all draw it in, because the whole value of a stock symbol is that it is stock.
`km_display::icon`'s test holds the claim that matters: outside the badge's own corner the mark is
**byte-identical** to the machine's, so the two read as one machine started two ways rather than as
two products.

**Two waves, not three.** A third has to reach past the letters' baseline, and a badge overlapping
the M is a badge drawn on top of the mark instead of beside it.

**It is for the launchers and the icon bar, and nowhere else.** A Start Menu entry that passes
`--stream`, the Linux desktop entry's stream action, `KM Stream.app`, and the
notification-area or menu-bar icon a streaming run puts up — those are the places where the two ways
of starting the machine are offered or running side by side. The window icon, the favicons, the
`hicolor` entry the plain machine registers, the Plymouth logo, the phone launchers and the site all
keep the unbadged mark: there is nothing beside them to be told apart from.

**Below 32 pixels the badge is an amber corner rather than a glyph**, on the same terms the monogram
is a smear at 16: what it has to say at that size is *not the plain machine*, and it says it. The
Windows notification area is the one consumer that draws it that small.

**Windows carries both marks in one executable and macOS carries two bundles.** `build.rs` attaches
the plain mark at `winresource`'s default ordinal and the badged one at the next, so the shell still
draws the machine for `karaokemachine.exe` itself while the tray and the Start Menu entry ask for the
second by ordinal and by icon index. Nothing installs a second `.ico` beside the executable, for the
reason `km_webshell::with_icons` already reads the first one back out of the running process: one
copy of a picture is one thing to keep in step with `icon/`.

## The on-screen number pad

**Drawn only where there is no keyboard — every Android, televisions included — and
`display.number_pad` anywhere else.** Android has no keyboard and there is no other way to enter a
song number.

**Not on the desktop.** The other argument for a pad is that a real karaoke machine has a front panel
and that a pad *tells* somebody they may type a number where an empty screen saying "Enter a song
number" does not. On a desktop the keyboard is already in front of the person, so the affordance the
pad substitutes for is not the missing one, and the pad spends the left half of the idle screen to
point at something visible.

**A setting rather than a bare compile-time check**, because "Android" is standing in for "no
keyboard" and the two do come apart — a desktop touchscreen, or a machine worked only by a remote,
can ask for the pad and get it; the platform decides the default and nothing more.

**A television is included, and it is a physical remote that settles it.** `Keypad::move_focus` and
`activate` exist for exactly that remote, and a real Google TV remote drives the transport strip —
arrows move the highlight, OK presses what is highlighted. A song has been dialled on a Google TV
Streamer with the remote alone and no second device in the room, arriving as
`origin: {kind: "catalog", number: "2001"}` — dialled rather than started by demo mode, which is the
distinction that makes it evidence.

**With no pad on the idle screen there is nothing to focus and nothing to dial, so the machine could
not be asked for a song at all without a second device**: the remote reaches the transport strip
during a song and nothing whatsoever before one. A pad costs half an idle screen; no pad costs the
ability to start the machine.

**One measurement is kept although nothing reads it: counting touch devices does not work.** SDL's
`initTouch()` registers every input device matching `SOURCE_TOUCHSCREEN` **or `isVirtual()`**, and a
Streamer whose input devices are all keyboard, D-pad and gamepad reports **two**. So "is there a
touchscreen" cannot be answered that way, and only `SDL_IsTV()` can — which is an `unsafe` call, and
`karaokemachine` holds exactly one `unsafe` exception, the `SDL_main` export that is how Android gets
in at all.

**What is genuinely lost is a television's idle screen.** It draws a pad across half of it, and a
person driving that pad with a D-pad takes several presses to enter a digit. That is a worse idle
screen than a television could have, and a working machine rather than one that needs a phone.

**`display.number_pad` is not reachable from a phone**, being absent from `SettingsPatchDto` and from
the owner's page, and `settings.json` is app-private on Android. So a television's owner cannot clear
their idle screen. The default is the right way round for a machine that has to work out of the box;
making the choice reachable is separate work, and the obvious home for it is the owner's page rather
than a new API field.

**The transport strip is explicitly not covered by this** and appears on every platform: it names
actions (`REPEAT`, `KEY +`, `MELODY`) rather than duplicating keys somebody can already see, and it is
transient, so it costs an idle screen nothing. `display.keypad` remains the master switch over both.

## The transport strip names its keys

**`F1`…`F9`, fixed per command and printed on the buttons — on a desktop only.** `Space`, `N`, `R`,
`Q`, `M` and the rest are bound too, and nothing says so: the strip is transient by design, so the
only place those bindings could be learned is the source, and the result is a machine whose keyboard
works and looks as though it does not. A second binding per command fixes the discoverability rather
than the capability, which is why the letters stay.

**Fixed to the command and not to the position on screen** — `MELODY` is absent for a song that
declares no melody channel, so numbering what happens to be drawn would make `F9` mean one thing
during one song and another during the next, and nothing at all during the great majority of the time
when the strip has timed out. Numbering the table costs one dead key and buys a keyboard that never
moves.

**The strip, the bindings and the printed hints are one table**, `input::TRANSPORT_COMMANDS`, because
a hint naming a key bound to something else is worse than no hint — and two lists kept in step by hand
is exactly how that happens.

**Desktop only, and compiled out rather than settable**, which is deliberately not the answer the
number pad next door gives: that one is a setting because a pad is the *only* way to type on a device
without a keyboard, so a wrong platform guess leaves somebody unable to enter a number at all, whereas
a wrong guess here costs a line of text on a button that goes on working. On Android the hint would
name a key that does not exist. A key too small to hold two lines drops its hint and keeps its label.

## Nothing is drawn where a television will not show it

**A safe inset of five per cent of each axis, and everything in a corner sits on it.** A set
overscans: it shows less than the picture it is given, cropping a few per cent off every edge, and
nothing in the signal says how much. Five per cent is the safe area broadcast has always worked to.

**A fraction of each axis, and not one figure off the shorter side.** Overscan is a percentage of
each axis independently, so a single number is right on neither: on a 16:9 panel four per cent of the
shorter side is four per cent of the height and two and a quarter of the width. The narrow half is
inside what the set eats, which is how the build number in the corner came to be drawn without its
leading `v` on a real television, and the position bar entirely below the line.

The number pad, the transport strip, the build number and the position bar are what this moved. Every
other left-aligned thing on the screen already sat on the same five per cent, which is why the corner
was the only place it showed.

**This is not overscan *compensation*, and there is no setting for it.** A set that eats more than
five per cent has a mode that stops it doing so, and a machine second-guessing the television would
shrink its own picture on every set that behaves. What the inset buys is that nothing the machine says
is in the strip a set is entitled to take.

## What the machine draws over a song that brought its own words

**A video or MP3+G song's words are pixels in its own picture, so everything the machine draws for
itself stands on top of them.** Its author chose where every word goes and any row may hold one.

**The machine's own furniture is drawn at full strength, over such a song as over one it drew
itself.** The song number and title, the `artist · language` line, `next:` and the queue count. A
screen says four things about what is playing and a room reads them from a sofa for the whole of a
song, so the test they have to pass is that somebody across the room can read them, and a mark thin
enough to stand back off a picture is a mark nobody reads.

**Standing back is worth nothing if what stands back cannot be read.** A thinner ink costs legibility
twice over, and the second cost is the one that surprises: a light ink thinned toward a pale picture
loses its own contrast, and the dark ring that would otherwise carry it is thinned by the same
fraction at the same time. Both halves of the mark give way exactly where the picture behind them
gives the least help.

**What keeps the furniture off a song's own words is where it is put, not how faintly.** A karaoke
picture carries its words along the bottom, which is where a disc has always put them, so the machine
claims the top-left column and a corner and draws nothing of its own down there. The one mark that
does go low is the position bar, and it visits rather than stands.

**The ring is what carries a row over an arbitrary frame**, which is
[`How heavy a ring a face carries`](#how-heavy-a-ring-a-face-carries) and the reason the furniture is
outlined at all. Against a pale picture it is the whole of the contrast a near-white title has.

**Where the bar sits is not a choice about the words.** It rests on the bottom safe inset, which is
the lowest line a television shows, and lower is not further from the words but off the screen. So the
visiting is what keeps it off them and the position is only what keeps it visible. The transport strip
stands above it rather than over it.

The five things such a song does not have are
[`What a video song does not have`](song-sources.md#what-a-video-song-does-not-have) and
[`What an MP3+G song does not have`](song-sources.md#what-an-mp3g-song-does-not-have); this is what
the screen does with the same fact.

## How heavy a ring a face carries

**A ring is a fraction of the face it surrounds, not a count of pixels.** Every face on screen
carries the same share of its own size, so a row reads with the weight its glyphs were drawn for.

**One count for the whole screen is a count sized for the largest face.** Three pixels around a
92-pixel lyric is a thirtieth of it; the same three around the 28-pixel `artist · language` row is a
tenth, which closes the counters of `a`, `e`, `o` and `g` and leaves the row heavier than the title
above it while saying less.

**The lyric face is what the fraction is anchored on**, because the lyrics are the words a room
reads over an arbitrary photograph and their ring is the one that has been judged against one. They
keep the ring they have at every screen height, and the rest of the screen follows from it.

**The floor is one pixel.** A ring that rounds away leaves text unreadable on a small window, which
is where it is needed most.

**The face answers for its own ring, and no caller passes one.** A style is chosen before a face is:
the closures the playing screen builds serve the title in one face and the two rows beneath it in
another, so a thickness settled when the style is made is a thickness right for one of them. The
drawing asks the font its size, which is the one place both are known.

## What is coming next stands at the top

**`next: …` is the third row of the top-left column**, under the song's number and title and its
`artist · language` line, in the same face and from the same margin.

**Where the words are is what the top answers.** A video or MP3+G song's author put the words near
the bottom of the frame, because that is where a karaoke disc has always put them — so a line along
the bottom of the screen is not covering a row that might hold a word, it is covering the row that
does. Two sentences on top of each other make both harder to read whatever either is drawn at, which
is why the answer is a column of its own rather than a thinner ink. The top-left column is the
machine's own for three rows on every song.

**It says the same thing wherever it is.** This line is the only permanent answer a queued song gets
— it starts playing, or it appears here — so what matters is that it is on the screen and legible,
not which end of the screen it is at.

**Nothing takes it away.** It stood down for the transport strip while both were along the bottom and
the strip is centered over the whole of it; from the top it is out of the strip's reach, so the strip
now covers nothing and the line is up for as long as there is a song behind this one. The `F12` frame
panel is the one thing that draws over it, which is the rule that already stands: a panel somebody
switched on outranks an ambient row.

**It is still shortened, and by more while the developer marker is up.** A queue label is a title, an
em dash and a singer's name; a long one overhangs the right margin and SDL clips it. The marker is
right-aligned across the same row, so the line takes everything the marker's cap leaves — and a fault
somebody has to see outranks a line saying who is on next.

## The position bar answers the person in the room

**The bar visits the two ends of a song, and the words have the screen between them.** Thirty seconds
at each end of a song whose words the machine draws, six at each end of one that brought its own. A
song's length is what a room wants to know as one begins, how much is left is what it wants to know as
one ends, and how far through it is what somebody who has just pressed a transport key is asking. What
the middle buys is the middle: a mark drawn through the whole of a song is one the room has stopped
reading and can still see.

**Thirty against six is the difference between the two rows.** The row the bar takes on the machine's
own screen holds nothing else, so a window wide enough to answer a room costs that screen nothing.
Over a song that brought its own words the same row is a row of that song's words, whose author chose
what goes in it, and a visit there is as short as the one a key press buys.

**Any bound key raises it, not only the transport keys.** Every key on this machine is somebody
standing in front of it, and a bar that came back for `PAUSE` and not for `KEY +` would be a rule
nobody could learn.

**A song that has stopped keeps its bar for as long as it is stopped.** Its position stops advancing
with it, so neither window can answer for it, and a room that paused to fetch a drink is the room most
likely to be asking how far through the song it is. This outlasts the key press that stopped the song,
because the stopping is the fact and the key press was only how it was asked for.

**A song that cannot say how long it is draws no bar unasked.** A video container declaring no
duration leaves a bar with no length, which cannot move, and a row spent saying nothing is worse than
the row. A key press is answered even then: somebody who pressed something has to see that the machine
heard it.

**It is its own deadline and not the transport strip's**, which is the half of this easiest to get
wrong. Only a D-pad move and a touch raise the strip, so a desktop keyboard raises it never and a bar
that followed it would be absent from the machine somebody is sitting at. Raising the *strip* on every
key press is the other tempting answer and is worse: the strip covers the picture's lower third, which
is more of the words than the bar ever took.

**A machine with `display.keypad` off keeps the bar.** That switch decides whether there are touch
targets at all, and a machine with none still has a song whose length the room wants to know.

**A seek from a phone does not raise it.** The remote that asked draws its own bar and its own elapsed
time, and the person holding it is not the person this is for. What a phone seek moves is the song, so
a seek into the last window brings the bar up with nobody in the room having asked, and a seek out of
one takes it away.

## Installing a package by dropping it on the window

**Drag a `.kmpkg` onto the machine's window and it is copied into the packages folder, installed, and
reported on screen.** The other routes to the same `Catalog::install` all assume somebody who already
knows where things are: a path in `settings.packages`, a `POST` naming a path, or a file copied into a
folder they first have to find with `--show-paths`. A file manager is what the owner already has open
the moment they have built a package.

**The file is copied rather than installed where it lies**, which is the one real decision here and is
the opposite of what the API route does. Installing in place would write `…\Downloads\vol1.kmpkg` into
`settings.packages` — a standing obligation that fails on the next start once the file is tidied away,
and one nothing forgets on its own, because `forget_packages_that_are_gone_for_good` deliberately only
forgets temporary paths. The packages folder is the answer to *where do I put my songs?*; a drop is a
way of answering it without a file manager, not a second place for songs to live. A package already
sitting in a scanned folder is installed where it is, since copying it would leave two files of one
package for the next scan to find.

**What it is called where it lands is the manifest's answer and not the dropped file's** — see
[`What an installed package file is called`](packaging.md#what-an-installed-package-file-is-called),
which is where the naming, the replacing and the sweep are argued. Copied to a `.part` name and
renamed, so an interrupted copy cannot leave something that looks like a package in the folder the
machine reads at every start.

**It is not a new permission.** The drop is not an HTTP request and meets no check at all, on the standing
reasoning that somebody standing at the machine with a file in their hand already has the disk — the
same position `--play` takes. *A package is a document* adds a fifth route, and inherits this
paragraph on its cold path and departs from it on its warm one.

## A package is a document

**Double-click a `.kmpkg` and it installs.** The routes into a catalog all assume somebody who knows
where things are, and the drop that closes that gap needs a machine already up with its window in
front of you. A file manager is what somebody has open the moment a package finishes downloading.

**The two cases, and both documents meet the second — they answer it differently because they are
different kinds of document.** A `.kmpkg` is a payload: what somebody wants is the songs in the
catalog, so a machine already running takes it and flashes it on the screen they are looking at. A
`.kmbuild` is a workspace, and the corpus a second double-click names need not be the one the running
window holds — so the second process says something is already listening and stops, leaving that
window's own Open page to open the other folder. See
[`A corpus that will not open is a page`](curation.md#a-corpus-that-will-not-open-is-a-page-and-a-failure-before-the-page-is-a-window).
Either way a second process cannot take the catalog's SQLite or the API port, so the double-clicked
machine asks first:

- **Something is answering on loopback.** Copy the package into the packages folder, then hand the
  *destination path* to `POST /api/v1/packages`. The running machine installs it and flashes it on its
  own screen, which is the screen the person is looking at, and the second process exits without
  starting anything. Starting one anyway would fail on the lock or bind nothing and sit there, and
  neither failure names its cause.
- **Nothing is.** Copy it in and start normally. A document open is an application launch, and the
  ordinary startup scan installs what is now in the folder.

**`GET /api/v1/discover` is the probe, not a bare connection.** A sweep meets whatever happens to be
listening on 8177, a router's admin page included, so the `app` field is checked and anything else is
treated as nothing. It is public unconditionally — it sits outside `/api/v1/admin/`, and always will —
which is what makes the probe work on a machine whose owner has set a password.

**The copy runs first on both paths, through the same `place` the drop uses.** Not a convenience: it
is what makes the hand-off legal. `POST /api/v1/packages` takes a path on the machine's own disk, so
handing over `…/Downloads/vol1.kmpkg` would install a package the machine looks for again at the next
start and does not find. It also means the naming rules are stated once, in
[`What an installed package file is called`](packaging.md#what-an-installed-package-file-is-called).
A second copy policy here would eventually disagree with that one, and disagreeing means destroying
somebody's only copy.

**It is not a new permission on the cold path, and it is on the warm one — say so rather than let the
asymmetry look like an oversight.** The drop meets no check, on the standing reasoning that somebody
holding a file at the machine already has the disk. A cold double-click is exactly that. The hand-off
is an HTTP request and does meet `POST /api/v1/admin/packages`, which needs the admin password —
**and this is the one place that hurt when installing became an admin action.**

**So the warm hand-off signs itself in.** It is the same binary on the same box, so it reads
`admin_password_hash` out of the settings file it already owns and mints itself a token. That is not
a hole: anybody who can read that file can read the whole data directory, which is the same standing
reasoning the cold path rests on. What it avoids is a double-click that works when the machine is off
and fails when it is on, which would be the least explicable behaviour in the product.

A hand-off that fails anyway leaves the package in the folder so the next start takes it. A failure
there is a delay, not a loss.

**Per platform, and the differences are the same three `A corpus is a document` names.** Windows
writes four values under `HKCU\Software\Classes` and the installer offers it as a task; Linux writes a
desktop entry and a shared-mime-info definition, into `~/.local/share` from `--register` and into
`/usr/share` from the `.deb`. **macOS needs no Rust at all**: the type is declared in the bundle's
`Info.plist`, and SDL3's Cocoa backend turns `application:openFile:` into `SDL_EVENT_DROP_FILE` — the
event the machine handles for drops. The `tao` delegate the package builder relies on has no
counterpart here and needs none. **The two phone platforms declare it the same way and neither
writes anything**: iOS in the bundle's `Info.plist`, Android in its manifest.

**The cold-launch case on macOS is not verified and is recorded as such.** `Cocoa_RegisterApp` calls
`[NSApp finishLaunching]` before installing SDL's delegate, and that is what AppKit documents as
dispatching the launch Apple Event — so a package that *starts* the machine may be dispatched before
anything implements `openFile:`. If it is lost, macOS supports this only for an already-running
machine, and `argv` is not a fallback there. The other two platforms pass the path in `argv` and are
unaffected.

**A registrar of its own rather than a shared crate**, and this is the second caller — `km-osopen`'s
header records the rule that a third earns the crate. What actually differs is most of it: the
extension, the ProgID, the MIME type, the UTI, the description, the icon set, the category list and
what the program does with the file. What is identical is `windowed_twin_of`, nine lines, which
matters more than its size: `--set-password` lives on the console twin, so that is the executable
somebody has in hand when they type `--register`, and registering it would open a console window on
every double-click.

### On Android the document is a stream, and it is the only route in without a cable

**Opening a `.kmpkg` from a file manager installs it, and on this platform that is not a
convenience.** Android 11 closed `/Android/data/<id>/files/` to third-party file managers and hides
it over MTP, so the packages folder can be written by `adb push` and by nothing a person has to
hand. A device with no cable had no way to put songs on itself.

**The type is claimed twice, and the broad claim is the one that works.** Android has no MIME entry
for `.kmpkg` and gives an application no way to add one, so a provider asked what a package is
answers `application/octet-stream`; that is what Downloads, the Files app and a browser's download
notification all send. The narrow filter names `application/x-km-package` — the type Linux already
gives the extension, so the string is written once — and fires where a provider happens to know it.

**The cost is that the machine is offered for any unknown binary, and it is paid rather than
avoided.** There is no narrower way to claim an unregistered extension over a `content:` URI:
`pathPattern` cannot say *ends in `.kmpkg`* for a name with a dot elsewhere in it, and a `file:` URI
is not something one application on this platform may hand another. A file picked by mistake is
refused on its name before a byte is read, which is what a dropped `.mp3` already gets everywhere
else; what it costs is a full-screen launch to be told so.

**The copy is the shell's, and so is removing it.** A `content:` URI is a stream and not a path, so
the shell reads it into a folder beside the packages folder and hands over that path — after which
the ordinary drop route copies it in under the name its manifest implies, installs it, and says so
on screen. Nothing in the machine is specific to this platform. The staging folder is swept by the
side that wrote it, because the machine never deletes a file somebody dropped and a copy it did not
make is not its to keep either.

**The launch intent is taken away from SDL before it is handed one.** SDL's activity reads the
intent's path itself and sends it as a drop, which is right for a `file:` URI and names nothing on
disk for a `content:` one — so a successful open would be answered by a refusal about a file that is
not there. Replacing it with a bare `MAIN` before handing over keeps the vendored SDL Java stock,
which is what the next upgrade of it wants.

**A second package opened while the machine runs is a second install, not a second machine.** The
activity is `singleInstance`, so that arrives where SDL overrides nothing and would otherwise be
dropped in silence. It needs none of the warm hand-off the desktop does: there is one process, and
it is already holding the catalog.

## A drop says what happened, and a fault does not

**A band across the top of whichever screen is up, in three colors, that goes away on its own.**
Distinct from the standing package notice, and the distinction is what each is *about*: a notice is a
fault in the catalog, true until somebody fixes it, and it belongs to the idle screen because that is
where the catalog is the subject; this reports something that has just happened, so it has to appear
over a song as readily as over the idle screen, and it has to leave.

**Three states rather than two** — a large package takes seconds to index, and a report that appeared
only once it had finished would make the interval between dropping a file and anything happening look
exactly like a file that was not accepted.

**A full-width band rather than a floating panel**, because the playing screen writes the song's title
into the top left and its badges and queue count into the top right on the very row a centered message
would land in, and something between the two reads as part of one of them. It takes the notice's slot
on the idle screen and the notice is suppressed while it is up, for the same reason a dialled title is
never drawn beside an error: two sentences in one place read as one.

**The band abuts that row rather than covering it**, and the distinction matters now that a filled
shape stands there. `small_px * 2` comes to two pixels below `0.05h` at every screen size, so the
title, the badges and the queue disc all hang below it and always have — which is *why* the notice is
suppressed by the caller instead of being left for the band to hide. The disc is not suppressed, for
the reasons in
[`How many are waiting stands in the corner`](#how-many-are-waiting-stands-in-the-corner): it is a
standing counter rather than a sentence, and the band crosses only its apex.

**What the notice's half of this says has changed** — a count and an area rather than a quoted reason
— but the split this entry draws has not, and reads truer for it: a drop says what happened, and a
fault says only that there is one. See
[`A fault says how much is wrong, not what`](#a-fault-says-how-much-is-wrong-not-what).

**Success is `accent_alt`**, the green no other screen draws — reporting a package that installed
perfectly well in the red reserved for faults would teach that color a second meaning.

## What the keypad line says, and how long it says it

**The line under the title is the machine's one place to answer a person standing at it, and what it
says there leaves by itself after eight seconds.** The digits being dialled, `no song 9999999` on a
number nobody has, `invalid number` on `000`, `nothing is playing` for a seek with no song, and every
refusal `update_settings` and `transport` can make, `no melody channel was detected for this song`
among them.

**That is the right place for all of it**: an answer belongs where the question was asked, and the
flash band above is about files arriving rather than about keys being pressed.

**Eight seconds**: longer than a drop's `installed …`, which is a result to notice, and shorter than a
drop's refusal, which is the only account anybody gets of why the songs are not there. Without a clock
the only things that clear it are the next digit, BACK and CLR — presses nobody has a reason to make
when the message is an answer rather than a prompt — so a refusal about a song outlives that song, the
next one, and the return to the idle screen.

**The digits are deliberately not on that clock**, and the two halves of one line getting opposite
answers is the whole point: a message is something the machine said and nobody is waiting on it, while
half a dialled number is something a person said and that person is still standing there.

**Nothing is said on success, and that is an invariant rather than a tidy-up.** Every message on this
line is drawn in `theme.alert` — there is no kind to choose by — so a confirmation here would arrive in
the color reserved for things going wrong. A queued song has more to show for itself than a message
does: it starts playing, or it appears as `next: …`, which is **permanent** where this line lasts eight
seconds. So every remaining thing on this line is a failure, and one color is *correct* rather than a
compromise. Adding a kind would keep the channel ambiguous and make the ambiguity somebody else's
problem to get right at four call sites. This is the judgment
[`Opening the packages folder from the machine`](#opening-the-packages-folder-from-the-machine) makes
one section down.

**And the size gives way before the words do.** Forty-four characters at 720p is wider than the
screen, so both ends are clipped and what is left reads as a fault in the machine rather than as a
sentence about a song. It walks the same four rungs a lyric line does, and then — unlike a lyric line,
whose next row is already spoken for by the next line of the song — it may **wrap**, to two lines, cut
with the same `…` the standing notice uses. Over a playing song the box grows for a message exactly as
it grows for a dialled title, because cutting the sentence to that box's width leaves
`no melody channel was det…`, which is a message about nothing.

## A fault says how much is wrong, not what

**`1 problem: packages`, or `4 problems: packages, sound`, and never the reason.** A refusal quoted
in full —
`"brasil-vol2" was not installed: 3 song number(s) already belong to another package: 500 is vol1's "Águas de Março"…`
— wraps to two lines and is cut with an ellipsis, and **the half that gets cut is always the
informative half**.

**A count and an area is the whole of what a television can usefully contribute.** Nobody fixes a
package from a sofa. What the screen has to do is get somebody to go and look, and *how much* and
*where* is all of that; the name of the file is detail for the surface that has a delete button
beside it. Three other surfaces carry the whole of it and one of them can *act*: `/admin/`'s
Problems tab lists every refusal and offers to delete the file behind each, `GET /packages` carries
it, and so does the online remote. See
[`A clash warns rather than only logging`](packaging.md#a-clash-warns-rather-than-only-logging).

**Both areas at once.** One sentence would mean one slot, so a package problem would beat a sound
problem to it and a machine with both would say only half of what was wrong — see
[`A machine that is not making sound with a real bank says so on the screen`](audio.md#a-machine-that-is-not-making-sound-with-a-real-bank-says-so-on-the-screen),
whose arbitration this replaces. Packages are named first, because songs being absent is the one
somebody can act on.

**It is counted per area rather than as a total and a set**, so a third area is one field, one arm
and one line in each catalog. There will be a third: `/admin/` already reports a wallpaper fault that
has deliberately never reached the television.

**The television's number is `/admin/`'s badge minus two, deliberately.** That page counts four
sources; this counts two. The picture fault is out by the standing decision above. The audio-device
fallback is out for a harder reason — answering it means enumerating the host's devices, and this
line is computed on a screen that redraws sixty times a second. Two surfaces counting one machine
differently has to be deliberate, and the difference is *which faults*, never the wording of one:
`SoundFontStatus::complaint` is the single sentence both use.

**A count and an area need no knowledge of what a demo or a SoundFont is, which is why they can live
in this crate's own catalog.** `km-display` is handed finished strings for `demo`, `soundfont_label`
and a song's language, because wording those needs a crate that knows what a demo and a SoundFont
are — and the price is that they arrive in English whatever
[`What language the television speaks`](#what-language-the-television-speaks-is-a-machine-setting)
says. A number and an enum carry no such requirement, so this line follows the machine's locale
where those three do not.

**It does not say where to look.** `3 problems: packages, sound` gives somebody standing at the
machine nowhere to go. Left out because the idle screen already carries the machine's address and QR
code in the bottom right, so the answer is on the same screen — and because a second line spent on
*and here is where* would be the line this change exists to reclaim. If it turns out people do not
make the connection, the fix is wording, not a new element.

## How many are waiting stands in the corner

**A filled disc with the count in it, top right, on both screens — and drawn at zero too.** The
number is otherwise only behind the `Q` key or the transport strip's `QUEUE` button: `next: …` names
one song at the top of a playing screen and says nothing about the four behind it, and the idle
screen says nothing at all. Somebody who has just dialled a number cannot see the queue took it
without opening the overlay, and a party cannot see it is six deep.

**Zero is drawn, which is the opposite of what the badges beside it do**, and the difference is what
each kind of thing is. Key, tempo and melody appear only when they differ from the default, because
each is a state that *changed during* a song. This is a standing counter, and one that vanished at
zero would leave *nothing is queued* and *this machine has no such display* looking identical — the
argument
[`What the idle screen says about the catalog`](#what-the-idle-screen-says-about-the-catalog) makes
about `No songs installed`, one row up.

**So the colour carries the state rather than the presence.** `accent` — the queue's own colour
everywhere else on the screen — when something is waiting, and `text_dim` when nothing is: at zero
the disc reads as furniture, and at one it lights up. `theme.panel`, the ordinary HUD backing, was
tried first and is wrong here for a reason worth keeping: it is `#0C0F18` at 78% over a `#0A0C14`
background, which is that background. Behind rows of text that invisibility is the point; as the
*whole* of a mark it leaves a `0` floating in the corner with no disc under it.

**The digits are not in the catalog**, for the reason the number pad's own keys are not: `4` is the
same mark in every language this renders, and an entry for one would only be a way to break it.

**The corner is the pill's, and the badge run moved left to give it up.** The alternatives were the
second row, where the bank label already is, and a new row above `0.05h`, where nothing draws because
the flash band comes down to it. The corner is where this screen has always put machine state, and a
counter is machine state.

**Taking that width away is what finally bounded the badge run.** It was the last right-aligned text
in the crate drawn with no cut at all, and it fitted a phone held in portrait by about forty pixels —
the third instance of the defect [`A lyric line that will not fit`](#a-lyric-line-that-will-not-fit)
and the keypad line each found before it. A latent bug that a layout change makes reachable is one
the layout change owns.

**It is not suppressed under the flash band.** The band ends two pixels below this row at every
screen size — it abuts the row rather than covering it, which is why the fault line is suppressed by
the caller rather than left for the band to hide — so it crosses only the apex of the disc, where a
circle is a few pixels wide. Hiding the count for the twelve seconds a refused drop is up would cost
more than that sliver, and it would make this the one overlay that took a standing fact off the
screen rather than covering it.

## Opening the packages folder from the machine

**`F10` shows it in the file manager — desktop only, and silent unless it fails.** The packages folder
is this product's answer to *where do I put my songs?*. The other routes in all assume something:
`--show-paths` and a file manager assume a command line, a path in `settings.packages` and a `POST`
assume somebody who already knows where the folder is, dragging a package onto the window assumes the
file is already in hand, and `A package is a document` assumes a file manager. A key that opens the
folder assumes only that somebody is standing at the machine, which is the position `--play` and the
drop already take about what a person at the keyboard is allowed to do.

**It is not a settings surface** and does not reopen `Front end scope`: it opens a folder the machine
already prints, already scans and already copies dropped packages into, and it changes nothing.

**The strip cannot grow to a tenth entry**, and that is the real cost of `F10`, `F11` and `F12`
rather than the muscle memory. A tenth `TRANSPORT_COMMANDS` row would be a button drawn on the
screen that no key can press — a silent fault, which is why `input.rs` asserts the table's *length*
rather than merely asserting that these three keys are not in it. This is deliberately **not** a
tenth entry: that table is the strip drawn over a playing song, and a button labeled `SONGS` among
`PAUSE` and `KEY +` would be the one control there that is not about the song.

**Compiled out rather than settable**, which is the answer `The transport strip names its keys` gives
and not the one the number pad gives: a wrong platform guess here costs one key, where a wrong guess
about the pad leaves somebody unable to enter a number at all. On Android there is nothing to show a
folder in, and songs reach a device through the *public* directory anyway. **The Debian appliance is
not excluded and does not need to be** — a box with no desktop has no `xdg-open`, so the press comes
back with the reason, and a constant that cannot tell a bare TTY from a Linux desktop should not be
pretending to.

**Silence is the success case.** A folder that opened is a file manager window standing in front of
the machine, which says so better than a band over the lyrics could; a failure gets the red band,
because it is the only outcome with nothing else to show for it.

**The singular folder, not every scanned one** — the one `--show-paths` calls the answer to the
question, rather than Android's public directory and whatever the owner named in
`settings.package_dirs`; one key opening three windows is a surprise, and somebody who set those knows
where they are.

## F11 opens the remote in a browser

**`F11` hands the machine's own address to whatever this computer uses for web pages — desktop only,
and silent unless it fails.** It finishes a question the connect panel raises and cannot answer. `I`
puts the URL and a QR code on the television, which is exactly right for somebody holding a phone and
no answer at all for somebody sitting at the machine: they would have to read a URL off a screen and
type it into a browser running on the same box.

**The panel says which key it is, on Windows and macOS.** A key nothing mentions is a key nobody
presses, and everything else on that panel is addressed to a phone: `F11 opens the remote in a
browser` is the one line on it for whoever is sitting at the machine. It is drawn beside the code and
under the address count, in the same dim face the count is in.

**It stops at those two operating systems, and the key does not.** `F11` is worth *attempting*
anywhere there might be a browser, and a press that fails says why on the screen; an offer made
before anybody presses anything is a different promise. The machine cannot tell a Linux desktop from
the appliance — the constants that gate `F10` and the file manager record why, and this one leans on
the same fact — and the appliance has no `xdg-open`, so a Linux line would be broken on half the
installations that read it. Windows and macOS reach `ShellExecute` and `open`, neither of which can
be missing. **A failed press explains itself and an absent line does not**, which is why the
asymmetry runs this way and not the other.

**The line comes out of the panel's three small rows rather than adding a fourth**, and that is the
shape of the problem rather than a saving. All three panel sizes are anchored under the lyric band
and none has room to grow: the ultrawide leaves the standing card nine pixels against a row costing
forty-four. The three states that fill all three rows are the three failures, and none of them
carries this line — a panel that does is a reachable machine, whose detail is a count and a PIN. So
the detail is wrapped first and this takes what is left.

**Whole or not at all, where every other capped block on this screen is cut with an `…`.** Those can
be cut because the reader can go and find the rest; this is the only instruction on the panel, and
`F11 opens the remo…` spends a row to name no key. The portrait phone is where that bites, nine
characters filling a line there, and it is the screen that already cuts the loopback sentence.

**The standing card does not carry it**, for the reason it carries no sentence at all: it is a code,
its address and padding, with no row to share and nowhere to put another. Somebody at the machine
during a demo song presses `I`.

**`F11` is the fullscreen key almost everywhere, and taking it is the cost of this rather than an
oversight.** This application's fullscreen is `F`, with `Escape` to leave, so the convention was being
honored by a key that did nothing. Nothing is taken from anybody, because `F11` was never bound. What
it does is open a window — visible, harmless, and undone by closing it. And a key reserved for a
convention it does not implement teaches nobody that convention; it just sits there.

**Loopback, not the address on the television.** The panel names an address a *phone* can route to,
which is a different question from the one a browser on this box is asking, and the two answers differ
in both directions. On the shipped `0.0.0.0` the listener is on loopback too, so `127.0.0.1` reaches
it without touching the network — and reaches it on a machine with no network at all, where the panel
correctly says *No network* and has no URL to offer. But a machine bound to one specific address has
**nothing** listening on loopback, so `--api-bind 192.168.1.42:8177` must be opened at that address.
`connect::own_url` is those three cases and is tested as such.

### On macOS the key is `Ctrl+F11`, because the bare one never arrives

**macOS keeps `F11` for *Show Desktop*, in the window server, so the press is never offered to an
application.** On an Apple keyboard left at its defaults it is Volume Down as well. A panel naming
`F11` there is a panel naming a key that does nothing — on the one platform the line exists to
serve, since everything else on the panel answers a phone.

**`Ctrl+F11` is bound on every platform, and only the panel splits.** Two spellings of one binding
rather than two bindings: both press `OpenRemote`, both work everywhere, and what a platform decides
is which of them it is willing to promise before anybody has pressed anything. The alternative was a
macOS-only swap, which puts a fact about one operating system inside the keyboard table — and
`km-display` draws for five of them and holds an opinion about none. `ConnectInfo::browser_key` is
the seam that already exists for exactly this, and it now carries the key rather than a yes.

**The cost is a second spelling on Windows and Linux that nothing announces**, which is the smaller
of the two costs on offer. It is reachable, harmless and does what the bare key does; the panel goes
on naming one key, so nobody has two things to learn.

**It joins `Ctrl+F10` on the shelf the modified function keys otherwise keep for debugging.** Both
are carried by their pairing rather than by the digit: `F10` shows you where songs go and `Ctrl+F10`
picks up what you put there, and this is the same key as the one beside it.

**Two refusals, and they are different failures deserving different sentences.** With
`api.serve_remote` false, `/` is the API's diagnostic landing page — so the key says the setting's name
rather than opening an index of endpoints in front of somebody expecting a list of songs. With no
server at all, it carries the same words the connect panel shows for a port conflict, rather than
inventing a second phrasing for one fault. **Silence is the success case**, as it is for `F10`.

## F12 draws what the frame meter measures

**`F12` puts the frame statistics on the screen, and `--frame-stats` puts them in the log.** They are
the same numbers over the same second, and the two switches are deliberately separate: turning the
panel on must not start writing to somebody's journal, and asking for the log must go on working with
nothing on screen. The meter is built when either asks; the flag decides only whether it also writes a
line.

It exists because the diagnostic was in the wrong room. *The screen looks choppy* is reported from a
sofa, and a line in a log on a box under the television is reachable over ssh, from somewhere else,
after the stutter has stopped. The person who can see the fault and the person who can read the
measurement were never the same person.

**A diagnostic gets a key without reopening `Front end scope`**, and the test that row implies is the
one this passes: it states nothing, changes nothing, and reports only what the machine is already
doing. The drop is named there as an exception because it *installs*; this is closer to the connect
panel, which also draws a fact about the machine and is not a settings surface either.

**No platform gate**, unlike the two keys beside it. Those ask whether there is a file manager or a
browser to reach; there is nothing outside the machine here, and a build with no keyboard simply
cannot press it.

### `F12` is one way in, not the only one

**Three pages carry a switch for it: `/admin/`, `km-admin` and `/dev/`.** *"A build with no keyboard
simply cannot press it"* was written as an acceptable limit and was the wrong reading of this
decision's own argument. The reason the panel exists is that the diagnostic was in the wrong room —
*the screen looks choppy* is reported from a sofa while the measurement is in a log reachable over
ssh from somewhere else. **The appliance is the sharp case of that, not an exception to it**: it is a
box under a television with no keyboard at all, so the machine that most needs this had no way in.

**`GET /api/v1/performance` and `PUT /api/v1/admin/performance`**, the public-read and admin-write
split every switch on that pane has. Under `/admin/` because it draws over the picture in front of a
room; that it *states nothing and changes nothing* is what earned the key its place without
reopening `Front end scope`, and is not an argument for a stranger on the LAN doing it.

**Nothing is written down, and that is the property this rests on.** A diagnostic that survived a
restart would be a panel somebody left on for a month, and a `settings.json` entry would make it a
setting — which is exactly what this decision says it is not. So it is one flag in memory, and the
key and the three switches move the same one: two answers would be two answers.

**It takes effect at once, unlike the two switches beside it on those pages**, which decide which
routes get mounted and so wait for a restart. Each page says which is which, because a pane holding
three switches that behave two different ways has to.

### What the panel says about a song is a diagnostic and not a description

**A second block names the gain in force and where it came from, the corrections applied to the
song's own events, the convention its words arrived in, and what the parser could not read.** The
title, the artist, the language and the length are drawn in the corner above it, and a panel
repeating them would answer a question nobody asked twice.

**It is the same argument one room further in.** *The screen looks choppy* put the frame meter here;
*why is this one so quiet* and *why does that channel sound wrong* are asked from the same sofa, and
the answers were a `debug!` line in a log on the box under the television. One key and one panel,
because two diagnostics behind two keys would be two things to remember about a machine nobody reads
a manual for.

**The test for a row is whether somebody would act on it.** A gain of 0.71 says why a song is
quieter than the one before it; a muted channel says why an instrument is missing; a truncated track
says why the words stop before the audio does. A track count, a ticks-per-quarter and a suitability
score describe the file and change nothing anybody would do, so they stay in `km-pack inspect`, where
a file is read in detail and in bulk.

**The gain carries its derivation, because the number alone cannot answer the question it is drawn
for.**
A song already at the reference, a song nobody asked to level and a song nothing could measure all
read `1.00`, and they want different answers. Four states, and the fourth is a machine with no bank
to level against.

**The position is drawn, as `m:ss` against the length, for every kind of song.** It is the one
row that describes the song rather than what was done to it, and it passes the test anyway: *the
words stop at 2:13* is what somebody reads off to find the fault again or to report it. The position
bar shows only near the two ends of a song, so without this row the middle had no number at all.

**A value that does not apply draws no row.** A video song has no channels to correct, no tracks and
no karaoke convention, so its block is four rows. This is the rule the frame counters already
follow: a nought meaning *not applicable* reads as *nothing went wrong*, and those are not the same
statement.

**A repaired note on its own is not damage**, which is the cut the log already makes. A note-off
synthesized in an otherwise well-formed file is the parser doing its job on one file in thirty-four,
so it is drawn as detail beside a truncation and never alone — and the panel and the log therefore
cannot disagree about whether a file is damaged.

**Nothing here is written down either**, and nothing is measured that the machine was not already
measuring. The block appears with the panel and is absent on the idle screen, where there is no song
to describe.

## `Ctrl+F12` holds the transport strip still

**The strip's six seconds are a feature everywhere except in front of the person changing it, and
`Ctrl+F12` is where that person gets them back.** It sits over the lyrics, so it goes away; anybody
working on its layout, its labels or its hit-testing is pressing a key every six seconds to look at
the thing they are working on, and measuring a button they can see for about as long as it takes to
reach the mouse.

**Beside the meter it sits under.** `F12` draws what the frame meter measures and `Ctrl+F12` stops
the strip vanishing from under whoever is reading it — the same pairing argument `Ctrl+F10` makes
about the folder, and the modified function keys are already where the diagnostics live. The digit
is not the point: if the function row is rearranged, this follows the frame meter.

**The pin beats the timer and does not consume it.** Unpinning hands back whatever the deadline had
left rather than dropping the strip on the spot, and a press while pinned still pushes that deadline
out, so a strip that was up stays up for the usual six seconds after the pin comes off. A key that
had to be pressed twice to be usable twice would be the wrong key for the job it exists to do.

**Nothing is written down**, which is the property
[the frame panel](#f12-is-one-way-in-not-the-only-one) rests on and this rests on for its reason. A pinned strip
that survived a restart would be a bank of buttons somebody left over the words for a month, and a
`settings.json` entry would make it a setting rather than the diagnostic it is.

**One way in, unlike `F12`**, and the difference is which room the fault is in. That panel is
reachable from three pages because *the screen looks choppy* is reported from a sofa about a box
with no keyboard, and the appliance is the machine that most needs it. This is for somebody editing
the strip's own code with the machine in front of them; there is no second room to reach it from,
and an admin switch would be a control on a phone for a problem nobody has on one.

## The machine says on its own screen when it is in developer mode

**A marker in the top right, third row, under the bank label, for as long as debugging mode or the
development console is on.** In the same `alert` color the standing fault line uses, on **both**
screens, and never conditional on anything else being drawn.

**What it is for is that neither state left a mark anywhere a person in the room could see.**
Debugging mode lets anybody on the network play a file off the machine's disk; the console goes
further and serves the whole API a second time with **no password on any of it** (see
[`The development console has an API that needs no password`](api-and-network.md#the-development-console-has-an-api-that-needs-no-password)).
Both are switches somebody turns on to get something done and then has no reason to think about
again. Every other account of them is on a page somebody has to go and open.

**The corner is the one `Which bank is playing` already argued for** — that is where this screen puts
machine state, rather than anything a singer did — and the other three were not available, which is
worth writing down rather than re-deriving. The bottom right is the connect panel's for the length of
a song, the bottom left is the demo line and the timeline, and the top left is the title on one screen
and the frame panel on both. This band is the one place a standing fact fits on both screens without
covering something.

**It shares that band with two things drawn from the left margin, and clears neither by a gap.** The
frame panel starts at 0.16 and runs down past this; `next:` is 0.17 exactly, the same row seen from
the other side. What keeps all three apart is that this one is right-aligned and capped at a little
under half the width, and the line opposite takes what that cap leaves. Worth stating because the
panel and this are on screen together exactly when somebody is diagnosing something — and because a
test looking for a vertical gap would be looking for something that does not exist and was never the
guarantee.

**Two states and not three.** *Debugging* and *the console*, and the console implies debugging — so
"console switched on, debugging off" is not a state anything is open in. It is a switch waiting for a
restart, and the admin pages are where that is explained.

**Neither of the two says anything about a password.** `DEBUG MODE ENABLED` and `DEV CONSOLE
ENABLED` name the state and stop there. The marker's job is that a switch cannot be left on
unnoticed, and a name serves that; what a switch *exposes* is a longer sentence than half a screen
width affords, and the admin pages carry it. A password clause on one marker and not the other also
invited the reading that the other had a password in place — which debugging mode does not promise:
it mounts the two `debug/play-*` routes, which are public whenever they exist.

**Worded by `km-display` from its own catalog, not handed in as a sentence.** The line `Which bank is
playing` draws is a *value* the caller holds and this crate could not name; two fixed states need
nothing from the caller, so this follows `A fault says how much is wrong, not what` instead. That
same line was moved across this boundary once before, and for the same payoff: a sentence resolved
outside arrived in English whatever the machine's locale said.

**The running state, not the stored one.** The pages draw a switch, and a switch has to show what the
next start will do; the screen has to show what *this* machine is doing, or the marker would appear
the moment somebody pressed a switch and before the surface it warns about existed.

**Zero frames is drawn as `measuring…` rather than as zeroes.** The meter reports once a second, so a
panel that waited for real numbers would come up blank for up to a second and read as a key that had
not worked. Shortening the meter's window while the panel is up was rejected because the drawn numbers
and the logged numbers would then mean different things, which is the one property this design is for.

**The four decode counters are drawn only when one of them is non-zero.** On every MIDI song three of
the four are structurally zero, because there is no decoder thread at all — so drawing them always
would fill half the panel with noughts meaning *not applicable* rather than *nothing went wrong*. They
are what catches the fault the timings cannot see: a display holding 60 fps exactly while the picture
has stopped, which is what was observed on the appliance.

**The top left, under the title rather than over it.** The playing screen writes the song's title and
artist into the top-left corner and its key and tempo badges into the top-right, and directly below
the title there is a band of nothing at all between the header and the lyric ladder. The panel sits in
it, at `0.16h` rather than at the margin — checked by rendering it rather than reasoned about. The
right-hand side was rejected because the badges are there with the bank label under them: three
things, against one.

## Leaving the app

**BACK goes up one level, and out from the top.** A number being typed is cleared; a loaded song is
stopped; from an empty idle screen the machine closes. Three presses leave from anywhere, every press
does something visible, and there is **no confirmation** — Android's TV quality guidelines require
that back exit without one and forbid gating it behind a prompt. Going up a level rather than quitting
outright is what those guidelines describe for back, and it means a stray press on a remote costs one
song instead of the evening. It matters on a phone as much as on a television.

## Leaving the screen stops the music

**The machine pauses when Android takes the screen away, and nothing starts a demo song while it is
gone.** SDL stops drawing when the activity stops — its Java parks the main thread — but the audio
device belongs to cpal, on a thread SDL has never heard of, so a song otherwise carries on. Measured
on a Google TV Streamer: **about seventy seconds of full-rate playback with the app on the home
screen**, after which Android's cached-app freezer suspends the process; reopening the app thaws it
and it **resumes mid-song**, which is how somebody walks in on a song that started the last time they
left.

**The freezer is not the answer, and relying on it is the actual mistake.** It is OS policy rather
than a contract, and it is conditional on precisely the thing this app would want to fix next: an app
holding audio focus is exempt from freezing. Behaving better on the audio-focus axis would therefore
*remove* the only thing stopping it — a machine that plays for ever instead of seventy seconds.

**Paused, not stopped, and it does not resume by itself.** Pausing keeps the song and the place in it,
so coming back finds the machine where it was, waiting for somebody to press play. Auto-resume was
considered and rejected: resuming on return is the same surprise as never having stopped, just better
timed.

**Off the screen is the one demo condition nothing bypasses**, a hand-pressed `Play something` from a
remote included — the watchdog thread goes on running while the activity is stopped, so without that
condition a backgrounded machine picks songs and plays them to nobody, and a trigger arriving from a
phone does not make an invisible song any better.

**Handled in an SDL event *watch*, not in the event loop**, and that is forced rather than stylistic:
by the time the app is in the background SDL has parked the thread that reads the loop, so the arm
would be read on return, minutes after it mattered. A watch runs synchronously on the thread that
pushed the event — Android's UI thread — so what it does is one atomic store, and the poll thread does
the pausing fifty milliseconds later. Blocking there is an ANR.

**Two things this deliberately does not do.** The output device is *not* handed back:
[`Holding the audio device`](audio.md#holding-the-audio-device) excludes `Paused` because reopening
would lose the position, which is the thing pausing was for — so a backgrounded machine leaves a
stream open, rendering silence. And it does **not** request audio focus, so a call, an alarm or
another media app still talks over a machine that is on the screen. That is a real gap on a phone,
known and unfixed, and a separate decision from this one: it is about sharing the screen, where this
is about not having it.

## Photographed wallpapers

**Allowed, and legibility-*measured*.** A photograph has to stay calm behind two lines of text for
three minutes, and `tools/cmd/assets/km-wallpaper-pack` settles that half: a **WCAG contrast ratio
computed in the band where the lyrics actually are**, so a hundred photographs are accepted or
rejected without anybody squinting at them.

**The gate is the app's own scrim**: an image ships when the darkening it needs is no more than the
45% the display already applies, and **the pack darkens nothing itself** — two scrims would fight, and
the second one would override a setting the singer is entitled to change.

**Provenance travels with every image** — provider, id, photographer, source page — and an
`ATTRIBUTION.md` is generated whether or not a license demands one. **The license is recorded per
image, not per provider**: a type that cannot hold a per-image license is a type that never prompts
anybody to look one up. See `Where a wallpaper pack's photographs may come from`.

**A pack is delivered as one zip to drop into the wallpaper folder**, which the app reads as a folder
of wallpapers; it does not become the app's index.

**A pack is never committed**, and the binding reason is not size: **a Pixabay or Pexels pack may not
be handed on at all**, their terms naming this exact product. Size would be a reason to fetch rather
than commit; the license is a reason not to ship it by any route.

**And it is not staged into a release by default.** `--zip-dest` defaults to
`local/assets/wallpapers`, the checkout's local asset overlay: `cargo run` picks a pack up with no
further step, and nothing else can see it. A default of `assets/wallpapers` would put it in every
carrier — the Windows folder, the macOS bundle, the tarball, `dist/bin`, the installer and the APK —
with nothing but the APK's size warning ever saying so.

**`--zip-dest ./assets/wallpapers` is how a pack goes into the next release**, and it is **not a
decision anybody may make for a Pixabay or Pexels pack.** The flag is for a pack whose every image
grants redistribution, and it is the license rather than the flag that decides;
`manifest.json`'s `redistributable` is how you know which kind you are holding.
`tools/dist/check-assets.sh` names any staged pack and its size on every staging run rather than
letting it ride along quietly. The cost of the default is that an overlay wallpaper folder replaces
the bundled one, so the shipped set is not shown beside a pack — see `Local assets in a checkout`.

## The wallpapers the machine ships

**CC0 or public domain photographs, as one zip.**

**CC0 rather than the CC BY the row above permits**, and the reason is reach rather than principle: a
file under `assets/` is copied wholesale into six carriers by `dist_stage_assets`, named by extension
in a seventh, and published in a public repository — an attribution obligation that has to stay
correct and visible in all of those is a maintenance surface with no upside, where a downloadable pack
can simply carry its `ATTRIBUTION.md` inside it. A `CREDITS.md` ships beside them anyway, naming each
image, its photographer, its source page, its license and the fact that it was cropped, resized,
blurred and vignetted — a courtesy under CC0, and the mechanism already in place if the set ever gains
an image under a license that demands it.

**A zip rather than loose files**, so the shipped set is the same *kind* of thing as a pack somebody
downloads and there is one mental model rather than two; `.gitignore`'s `/assets/wallpapers/*.zip`
admits exactly this one, because that rule was written when every possible zip was somebody else's
photographs.

**The `.deb` is the carrier that notices**: it lists `assets/wallpapers/*` by extension where every
other carrier copies the tree, so its glob is what has to change when the shipped form does.

**`km-display`'s `wallpapers` example stays and its output is not committed**: it is the way to make a
legible-by-construction set if a photograph ever has to be pulled.

## Where the owner's own wallpapers live

**A `wallpapers` folder in the data directory, beside `packages`, which replaces the shipped set
whenever it holds anything.** The sibling of `Where packages live`, reached by the same argument:
`assets/` ships with the build and is read-only where it counts — root-owned under `/opt` on Debian,
inside a signed bundle on macOS whose seal `codesign --verify --deep --strict` checks, re-unpacked
from the APK on Android. So "delete the shipped file and drop in your own" is not a mechanism this
product has on three of its four platforms; only the Windows installer, being per-user, would allow
it, and an update would put the file back.

Drop a pack in and yours are shown; empty it and the shipped set returns. Nothing under `assets/` is
ever touched, and a data directory survives the upgrade that replaces `assets/`. A pack, and not a
picture — see [`A wallpaper is a pack`](#a-wallpaper-is-a-pack).

**It replaces rather than merges**, because `Playlist` holds exactly one directory — the same accepted
cost `Local assets in a checkout` records for the checkout overlay. **Merging the three candidates is
refused**: first-match-wins is what makes "empty it and the shipped set returns" a sentence anybody can
hold. Naming extra individual **files** is a thing a list of directories cannot express at all, so it
is a second field rather than a wider first one — see
[`The wallpaper folder is chosen again, not once`](#the-wallpaper-folder-is-chosen-again-not-once).

**Empty means fall through, and that is load-bearing rather than tidy**: the folder is *created* on
first start so that it can be found, and an existence test would then replace the shipped wallpapers
with nothing — a black screen with nothing in any log. The same trap is latent in the checkout
overlay. `Paths::create`'s own comment says the asset directory is deliberately not created because
"creating an empty one would turn 'no assets installed' into a directory that looks deliberate"; here
an empty directory *is* deliberate — it is an invitation — which is exactly why emptiness has to mean
fall through.

**Resolved by rule and never by a setting**, as `Where packages live` decided: `wallpaper.dir` wins
outright over both, and `--show-paths` names the wallpaper folder and which of the rules produced it,
which is the fastest answer to "why am I not seeing my pictures". **That rule name is on the API too**,
with the same restraint: the folder's *path* is the operator's filesystem layout and stays off the
wire, while which of four rules won is publishable and is what the question actually wants.

## A wallpaper is a pack

**The wallpaper folder holds zip files and nothing else.** A picture loose in it is passed over, and
the upload routes take a `.zip` and refuse a `.jpg` rather than accepting a file the display would
then ignore.

**Because a picture has no identity and a pack does.** A zip is named once and stays named; two
copies of one are recognizable as one thing, so a pack sent again replaces the pack it rebuilds, and
`km-admin`'s `wallpapers-beaches-e36b9929.zip` says which search made it. A loose `IMG_1234.jpg` says
nothing at all — and a second photograph under that name is a file the machine cannot tell from an
update of the first, so it either destroys somebody's picture or shows two of them for ever. The
folder is the one that would fill with those, a camera roll being where its contents come from.
That is the same argument
[`What an installed package file is called`](packaging.md#what-an-installed-package-file-is-called)
makes about a `.kmpkg`, and it lands harder here because there is no manifest to fall back on.

**What it costs is a step, and the step is named.** An owner with a folder of holiday photographs has
to zip it before dropping it in, where before they could drop the pictures. That is one action in
every file manager on all four platforms, it is the action they would take anyway to move the set
between machines, and it buys a folder where every entry can be replaced rather than one where some
entries can only be duplicated.

**And it costs per-picture removal, which is the sharper half.** `Removing a wallpaper` takes away a
file, and the machine never unpacks an archive — so a picture inside a pack has never been removable
on its own and now nothing is. Taking one photograph out of a set is done where the set is made.

**`debug.wallpapers` still names a loose image**, and that is not an exception smuggled in: naming an
individual file is the fragility `debug.` exists to hold, exactly as `debug.packages` names a
`.kmpkg` the folder rule would not have found.

**The shipped set was already one zip**, `assets/wallpapers/default-wallpapers.zip`, so nothing that
ships changes.

## Zipped wallpapers

**A zip file in the wallpaper folder is a folder of wallpapers.** Any number of them, each holding any
number of images at any depth, and every one of those images is an image in the folder — same cycle,
same count, same shuffle, named `pack.zip/beach.jpg` so it is clear where it came from.

Wallpapers arrive as collections and this is how a collection arrives: one file to drop in, one file
to remove, nothing unpacked to disk and nothing for a copy to leave half-finished.

Only the archive directory is read on a scan, so the rescan that lets images appear while the machine
runs stays cheap; an image is decompressed only when it is about to be shown. A corrupt or encrypted
archive contributes no images and is reported, rather than costing the folder its other pictures.
`__MACOSX` AppleDouble entries are skipped — they carry image extensions and are not images.

**Zip and no other archive** — it is what image collections come as, and reading it costs a dependency
the packager already had.

## The pictures come in a random order

**A pass over the folder, not a draw each time.** Every picture is shown once before any is shown
twice, and the order is made again when the pass ends. Picking independently at each change is the
other thing "random" can mean and it is the wrong one here: over a hundred pictures it shows one
twice in a row often enough to look broken, and leaves others out of a whole evening. What somebody
wants from a folder of photographs is to see the folder, in no order they can predict.

**The picture that closed one pass does not open the next.** A fresh order can put it first, and that
is the single repeat a viewer would actually notice, because the two showings would be adjacent. It
is moved, which costs one comparison at the one moment a pass ends.

**`wallpaper.shuffle`, on.** Name order is what a person gets by turning it off, and it is worth
having: a set built to be walked in order — a numbered sequence, a story — is a real thing to put in
the folder.

**The order is the machine's and the file list is not.** `/admin/pictures` lists *files* by name,
because it is a list somebody is looking a name up in, and one reordered every thirty seconds is one
nothing can be found in. The two orders are separate on purpose, and the page is the one that is
alphabetical.

**A picture added while the machine runs joins the pass that is running**, somewhere between the one
on screen and the end of it, rather than after every other picture in the folder. Appending is what a
list of a hundred and twenty makes of "drop a picture in and it appears": an hour of waiting. Nothing
is ever inserted in front of the picture on screen, which would move it and repeat it.

**A removed picture costs only its own place.** The rest of the pass keeps the order it had, so
deleting one file does not reshuffle an evening.

**The randomness comes from the operating system, and the ordering does not.** `km-display` takes a
seed and holds the algorithm, so a test pins an order exactly while the machine gets a different one
each start. A machine that could not get entropy uses a fixed seed and says so: an evening that opens
on the same photograph as the last one is not a reason to refuse to start.

## A song starting changes the picture

**The picture belongs to the song, not to the clock.** An interval alone puts a background behind a
performance for no reason connected to it: one song spans three pictures while the next gets the tail
of one, and a change lands in the middle of a verse where it reads as the screen twitching. A new
singer standing up is the moment a room will accept a new background, and it is the only moment on
this machine that means anything to the people in it.

**The interval stays and runs underneath.** An idle screen has no songs to mark, and that is where a
cycle earns its keep; the two triggers answer different halves of an evening. Every change restarts
the interval, so a picture that arrives on a song start gets a full one rather than being taken away
a second later by a timer that was nearly up.

**`wallpaper.on_song_change`, on.** The setting exists because a room that wants one steady image
behind an evening is a room with an opinion rather than a fault, and the default is on because the
trigger is the one somebody would have asked for.

**Every kind of song, video and MP3+G included.** Their own picture covers the wallpaper, so the
change happens where nobody can see it. The alternative is a song-kind test in
[`Machine::start`](../../crates/machine/karaokemachine/src/machine.rs), which is the one place song
kinds are otherwise settled, and it would buy a decode at the cost of a branch that has to be right
about every kind added afterwards. The picture also has to be right the moment the song *ends*: a
video finishing hands the screen back, and a wallpaper skipped for three videos would hand it back
the same image the room last saw.

**A song ending changes nothing.** The idle screen keeps what the last song was given, because a song
ending is not a song starting, and a change with nothing to mark is the interval's job.

**It reaches the display through the request flag `POST /wallpapers/next` already sets**, so a song
start takes the one change path — re-resolve the folder, rescan it, advance the playlist, ask the
loader — rather than a second route that would have to be kept in step with it. That path is
[`The wallpaper folder is chosen again, not once`](#the-wallpaper-folder-is-chosen-again-not-once),
and a song start is now what makes its rescan irregular.

**The television says nothing about it and `/admin/pictures` does.** A version number over a
wallpaper is furniture nobody across a room can act on, and so is a note about why the wallpaper
changed; the operator's page answers *when does the picture change* in one sentence, and the interval
on its own would be half of that answer given as all of it.

## A wallpaper pack is named, and the name is a label rather than part of the search

**`km-admin`'s picture search asks for a name, and it goes into the zip's file name after the
`wallpapers-` prefix**: `wallpapers-beaches-e36b9929.zip`. That name is also the pack's folder, the
id a route hands back, and what the machine calls it once it is sent. The zip's name is the whole of
a pack's identity everywhere it goes, which is what makes it worth asking for rather than a caption.

**A search will not run until the pack has one, and nothing suggests what it should be.** These are
two halves of one rule. A name nobody has to give is a folder of packs that differ by eight hex
characters, which is the failure below; a name the page proposes is the name every pack on every
machine then carries, which is the same failure with a word in front of it. So the field is required
where the search is started, and it is empty until somebody types in it.

**It is asked for where the answer is still known.** What a pack is *for* lives in nobody's records:
an hour after the build, the difference between two searches is a thing to be remembered rather than
read. The name box sits on the form the search button submits, so naming and searching are one act.

**A packager builds more than one**, which is the whole point of
`Everything it makes is kept, listed, and sent as a second act` in
[`distribution.md`](distribution.md#everything-it-makes-is-kept-listed-and-sent-as-a-second-act).
Unnamed, they are a list of rows differing in eight hex characters, where telling two apart means
remembering which search produced which.

**The picture count stays out of the name, and the hash is the whole of what is in it.** A count is a
fact about one build: the same search against a grown cache finds a different number, so a count in
the name gives that rebuild a *file of its own* — sitting in the wallpapers folder beside the first
and showing every photograph in both twice, `Playlist` keying a picture on its archive's name. A
pack's identity has to be the thing that does not move when it is built again, which is the same
argument
[`What an installed package file is called`](packaging.md#what-an-installed-package-file-is-called)
makes about a package's id. The count is in `manifest.json`, which is where the list rows read it.

**It is applied where a pack is *kept*, and never inside the pipeline.** `km-wallpaper-pack` writes
into a scratch directory it clears on every run and derives that file's name from `Config::hash`, so
putting the name into the config would make renaming change the hash — which invalidates
`analysis.json` and asks somebody to measure four thousand photographs again because they changed a
word. Keeping is where a pack stops being scratch, so keeping is where it acquires a name. The
command-line tool is unchanged and has no such option.

**The prefix stays in front of it.** Nothing now depends on that — the loose-pack sweep takes any zip
whose stem is a legal single path component — but every pack this program has ever written begins
`wallpapers-`, and one that did not would read as something else that happened to be in the folder.

**Slugged as it is stored, and shown back slugged.** Lowercase, one dash per run of spaces and dashes,
ASCII letters, digits, dots and underscores kept and everything else dropped — so the rule is
demonstrated in the field rather than described beside it. Accents are dropped rather than
transliterated (`Músicas` → `msicas`), the same judgment
[`One alphabet, everywhere`](songs.md#one-alphabet-everywhere) makes about a fold table: ugly and
unambiguous beats a table of somebody's opinions about other people's alphabets. A name that slugs
away to nothing is no name, which is also what a blank box means.

**A name must contain a letter or a digit.** A dot is legal in a file name so dots are kept — and
`..` is then made of nothing but legal characters while still being the parent directory. It would
be refused later, by the check that decides which folder a pack id may name, and a name silently
refused later is worse than one refused as it is typed.

**This is the second copy of the slug rule in the repository, and the boundary is what keeps it.**
The rule lives in `km_kmpkg::name_slug` for everything in the root workspace — the machine's package
file names and `km-package-builder`'s ids both ask it — and this is the copy on the other side of a
line, `tools/cmd/assets/` being a second cargo workspace that exists precisely so its members can
take `reqwest`'s TLS while `km-package-builder` does not. Sharing nine lines across that boundary
means another crate, published or path-patched into both workspaces, to hold a `match` over
characters.
[`One spelling per concept, across every surface`](foundations.md#one-spelling-per-concept-across-every-surface)
is answered where it can be and paid for here. It is spelled character-for-character like
`name_slug`, so the two stay comparable by reading, and each has a test naming the same examples.

**`karaokemachine`'s `bank_id` is not a third copy but a different rule**: it folds a dot to a dash,
where this keeps one, because a bank id goes in a URL and a package name goes in a file name.

## A running server has an icon in the bar

**Every program here that starts a web server and gets out of the way puts an icon in the OS icon bar
— the Windows notification area, the macOS menu bar — and that icon can close it.** So this is
`km-package-builder`, `km-remote`, `km-admin`, and the karaoke machine on a streaming run. A machine
drawing on a television is excluded, its screen being its own answer to *is this running*.

**What earns it is the runs with nothing else to show for themselves**: `--browser`, `--lan`, and the
fallback taken when a webview will not build all leave a server listening with no window and, on a
double-clicked GUI-subsystem executable, no console either — a process nothing announces and nothing
but Task Manager stops.

**Closing the window still quits**, and there is deliberately no minimize-to-tray: a program somebody
believes they closed must not be left running behind an icon they may not have noticed, so the
`Closing the window is the quit` position in `The package builder's window` and `The remote's window`
stands unchanged and the icon adds a case rather than altering one.

**The icon belongs to the executable, not to the window** — it is the windowed executable of each
pair, in every mode, and never the console twin, which has a console in the taskbar and a working
Ctrl-C and needs no second answer to a question already answered.

**And it rides on the existing `desktop` feature rather than getting one of its own**, so Linux has no
tray for the same reason it has no window: `tray-icon`'s only backend there links
`libayatana-appindicator` at load time, which is precisely the dependency that decision refuses for
`wry` — and this project's Linux target is a bare-TTY appliance with no icon bar to put one in. A
second feature would be a second thing for five staging scripts, `tools/setup/features.sh` and
`.cargo/config.toml` to keep in step, for a platform that cannot use it.

The menu is the address as a label, `Show window` where there is one, one entry for each page the run
serves, and `Quit`. `Show` is absent rather than grayed out when there is no window, because a grayed
command invites somebody to work out what would enable it, and a page the run does not serve is
absent on that same rule.

**A tool's entry is named for the act and the machine's are named for the pages.** A tool serves one
page, so `Open in browser` is unambiguous and the address printed directly above it already names
that page. A streaming machine serves three — the singer's remote, the screen the stream is on, and
the owner's page — and with three of them which one you want is a real question, so they are
`Remote`, `Watch` and `Setup`: the words the rest of the product already uses for those three doors.

**This is what a streaming machine has instead of a screen.** It draws on no television and opens no
window, so on a desktop it is a process nothing announces — and the three pages it serves are
otherwise reachable only by somebody who knows the addresses to type.

### The machine's icon names the address a phone can reach

**The address on the machine's icon is the ranked one the television's panel and the QR code name,
and loopback only where there is no such address.** Its three pages are for hands that are not on
this box: the singer's remote is a phone, and the watch page is a set in another room. An icon
naming `127.0.0.1` gives the person reading it nothing they can type anywhere, which is the whole
of what that line is for.

**A tool's icon names loopback, and that is the same rule rather than an exception to it.** Each of
the three serves one page, and that page is the one in the tool's own window, on this box — so the
address beneath it is the address of the thing somebody is already looking at.

**The machine's icon asks again; a tool's has no reason to.** A LAN address moves when Wi-Fi arrives
after the machine does, when a lease renews, or when a cable comes out, and a streaming run lasts an
evening. So the icon follows the machine's own re-resolution rather than naming what was true when
it started. **A machine with no address at that moment keeps the last one it had**, because a menu
entry opening a page that will not load says more than one that opens nothing.

### The icon in a macOS menu bar is a silhouette, and the run behind it takes no Dock tile

**A macOS menu bar draws template images, so the mark this product puts there is the letters with
the tile taken off.** The system reads a template for its alpha alone and paints the shape itself,
which is how one file comes out dark on a light bar, light on a dark one, and inverted while its
menu is open. Every other glyph up there is one. A full-bleed tile cannot be: its silhouette is the
tile, so it would draw a solid block — and drawn as it is, a near-black plate in a bar that is often
near-black too is a hole rather than a mark.

**It carries no badge, although it is only ever in a bar for a streaming run.** A badge separates two
things standing beside each other, and nothing else this program has ever puts an icon in that bar.
That is the rule the window icon, the favicons and the phone launchers already follow.

**The Windows notification area keeps the colored mark, and the difference is the bar rather than the
program.** There an icon stands among other full-color icons, is drawn from the executable's own
resources, and has no template convention to follow.

**A streaming machine gives up its Dock tile once its icon is in the bar, and keeps it when there is
none.** There is no window for a tile to raise, so a machine with an icon has a tile that opens
nothing; a machine without one has a tile that is the only thing announcing the run and the only way
to reach it. Giving it up before the icon exists would trade a small untidiness for the very fault
the icon is there to prevent — a process nothing announces and nothing but Activity Monitor stops.

**Set while the program runs rather than declared in a manifest**, because `LSUIElement` cannot say
*once the icon is there*. It is also the only spelling that works: the streaming launcher hands over
to the machine's own bundle, so the manifest macOS reads belongs to that one, and a key in the
launcher's own would be read by nothing.

**And a macOS launcher asks LaunchServices to start the other bundle rather than running the binary
inside it.** A process that execs its way from one bundle into another holds the identity it was
launched under and the one its new image belongs to at the same time, and a status item created
under that disagreement is handed back by `NSStatusBar` and never drawn. There is no error and
nothing in a log: the machine starts, streams and answers its API, and the menu bar stays empty. So
the streaming launcher `open`s `Karaoke Machine.app` and the machine runs as the one bundle it is.

**What that costs is the name on the run, and it is the right name.** The program in the menu bar and
in the app switcher is the machine, not *KM Stream* — the second bundle is a way of
starting the machine, not a second program, which is what
[`A badge says how the machine was started`](#a-badge-says-how-the-machine-was-started) already says
about its mark.

## The windowed tools have a menu bar on macOS

**On macOS ⌘Q is not a key a window is sent — it is a key equivalent on an item of the application
menu — so a program with no menu bar cannot be quit with it.** `tao` sets no menu, which leaves
`km-package-builder`, `km-remote` and `km-admin` with an empty bar and no ⌘Q. Each gets one. The
karaoke machine is excluded for a different reason than it is excluded from the icon in the bar:
SDL's Cocoa backend builds an application menu of its own, and its Quit already arrives as the
machine's ordinary quit event.

**Quit is this project's own menu item carrying the tray's id, not the platform's predefined one.**
The predefined Quit is AppKit's `terminate:`, which ends the process without ever asking the server
to stop — and asking is what these programs do on the way into an exit, so a `terminate:` would walk
past the shutdown and leave the remote sitting out its grace period waiting for an outcome nobody
started. Routed the ordinary way, ⌘Q is the *same* `Quit` the tray's menu sends and the same shutdown
closing the window asks for: a fourth door onto one exit, which is the position
[`A running server has an icon in the bar`](#a-running-server-has-an-icon-in-the-bar) already takes
about the third.

**There is an Edit menu, and it is not decoration.** The same empty bar takes ⌘C, ⌘V, ⌘X and ⌘A with
it, because those are first-responder actions AppKit routes through this menu — so without one, the
text fields on the page cannot be copied into or out of. The six standard editing items cost nothing
to add and ask nothing of the program: AppKit sends them to whatever has focus, which is the
webview.

**There is deliberately no ⌘W.** The platform's Close Window item closes the key window through
AppKit directly, which does not pass through the close handler every *closing the window is the
quit* position in this file and in [`curation.md`](curation.md) and [`remotes.md`](remotes.md) is
written against. It would be a second, untested way to end these programs, offered for a shortcut
nobody has asked for. A ⌘W that was wanted would have to be a routed command like Quit.

**macOS only, and not because Windows was forgotten.** Windows has an icon bar and no application
menu bar at all, and Linux has no window; this is the one thing in these shells that splits on macOS
rather than on Linux.

## A tool window opens in the middle of the screen

**All three windowed tools name a position, and it is the center of the primary monitor.** The
package builder, the offline remote and `km-admin` each already measured a screen in order to clamp
their opening *size*; naming the corner as well costs one more reading of the same monitor.

**A window that names no position is not neutral — it takes whatever the platform's cascade gives
it**, which on Windows is a staircase from the top left that walks further down with each run of the
same program. These three are opened, closed and opened again all evening: the builder once per
corpus, the remote whenever somebody wants the queue, `km-admin` whenever a machine needs something.
The staircase is what that habit produces, and the far corner of a wide desk is where it ends up.

**The monitor's own corner is added rather than assumed to be the origin.** On a two-screen desk the
left-hand monitor routinely reports a negative x, so centering on the screen's *size* alone puts the
window on whichever display happens to contain that coordinate — which is the fault this exists to
fix, arrived at from the other side.

**The primary monitor and not the one the cursor is on.** A pointer is where somebody's hand happens
to be, and a program started from a Start menu, a Dock or a file association was not necessarily
started by a hand at all. The primary monitor is the one the platform already treats as the default
place for a new window, so this is the platform's own answer with the cascade removed.

**Three copies of nine lines, which is the arrangement already in force.** `fit` and `opening_size`
were each written three times before this, deliberately: each tool wants a different `WANTED`, and a
shared crate for two pure functions would have to be reachable from both workspaces. `centre` follows
them, and each of the three has its own test asserting it — including the clamp that keeps a title
bar on the screen where a platform reports a display smaller than the window it gave us.

**What it centers is the page and not the frame**, because a window is positioned by its outer corner
and sized by its inner one. The window therefore sits a title bar's height low. Asking the platform
for a decoration height before the window exists is the alternative, and being a title bar out is not
worth it.

## Where a webview keeps its profile

**In the platform's per-user cache directory, one per program — never beside the executable, and never
under `assets/`.** A bare WebView2 builder uses its own default: a `<exe name>.WebView2` folder in the
executable's own directory. A folder somebody unzips and hands on should not start growing browser
profiles inside itself the first time each program in it is run, and `dist/bin/<platform>` is the
sharpest case, holding both of them side by side. An installed build is sharper still: a `{app}` under
`C:\Program Files` has nowhere writable to put one at all.

**`assets/` was the obvious parallel and is refused on the argument `Where packages live` already
makes** — that tree ships with the build and is read-only where it counts — and a browser profile is
accumulating per-user state of exactly the kind that row moved out of it.

**`cache_dir` and not `data_dir`**, because on Windows `directories` maps `data_dir` and `config_dir`
to the *roaming* `%APPDATA%`, and this is hundreds of megabytes of browser cache with no business
following somebody onto another machine or into a domain profile. It is also honestly disposable: both
pages are server-rendered htmx with no `localStorage`, so deleting the folder is a supported repair
rather than a loss, which is the property that makes the cache directory the truthful slot rather than
merely the convenient one.

**One each, not one shared**, which preserves the separation the two default folders already had and
stays clear of WebView2's documented refusal to start when a running instance holds the same user data
folder under different environment options — the two programs can be open at once, so that is a live
case. Each uses the `ProjectDirs` qualifier it already uses elsewhere, so neither gains a second
identity on disk.

**`--data-dir` deliberately does not govern it**: that flag names `km-remote`'s catalog copy and its
favorites, which are what a scratch run wants kept out of the way, and a regenerable browser cache is
not — tying the two would let a flag about somebody's collection quietly decide where a cache goes.
The accepted cost is that a `--data-dir ./local/...` run is not quite self-contained.

**Windows-only in effect, and the code says so rather than pretending otherwise**: only the WebView2
backend reads the field, WKWebView ignores it and keeps its state in the application's own container,
and Linux never builds the `desktop` feature — so a path returned on those platforms would create a
directory nothing would ever open.

## A link that leaves a webview

**The window denies the new window and hands the address to the platform's browser.** A webview is not
a browser and has nowhere to put a second window, so `target="_blank"` raises a *new window requested*
event and, with nothing answering it, the click does nothing whatsoever — no error, no navigation, no
tab.

**The two mobile shells over these same pages do this** at the native layer
(`shouldOverrideUrlLoading`, `decidePolicyFor`), so all three hosts agree that a link out of the tool
leaves the tool.

**Only `http` and `https` leave**: what arrives is whatever the page asked for and it ends up as an
argument to the platform's opener, which will open a file or a program as readily as a page.

**A new window asked for on the tool's own server stays in the window.** `km-package-builder` loads
it in place. A browser handed the tool's loopback address would be a second copy of the tool beside
the first, and the webview has no tab to give instead.

## A song's page and its similar names open in a tab of their own

**In `km-package-builder`, the song title and the ≈ button carry `target="_blank"`, beside YouTube's
♪.** Each is a look aside from a list somebody has spent a while narrowing, and a tab keeps that list
where it was. The page asks and the host decides: a browser opens a tab, and the tool's own window
loads the page in place under the rule above. No template tells the two apart, because the server
cannot. One process serves both the window and the browser tab *Open in browser* opens from it.

## Where *Open in browser* lands

**On the page the window is showing, not the address it started on.** The filter a curator has built
up lives in the address bar — htmx pushes it there on every change — and nowhere the server can see,
so opening `state.url()` after twenty minutes of narrowing a corpus opens the whole corpus again. Both
doors do it: the header button sends `window.location.href`, and the icon in the bar asks the webview.

It stays a server round trip rather than becoming an `href`: the URL wanted is the one the browser is
on *now*, which no template knows.

**What arrives is checked against the tool's own address before it is opened**, because it too reaches
the platform's opener; anything else silently becomes the plain address, since somebody who pressed
*Open in browser* wants a browser rather than an explanation.

## The number being typed shows its song

**The title and performer appear as the number is dialled, and a number nobody has shows nothing at
all.** A name arriving only *after* Enter makes the way to find out whether 10234 is the song you
meant queueing it in front of the room. Resolving as you type is what every commercial machine does
and what makes a mistyped digit recoverable.

**The half that had to be decided is the miss**: `1`, `10` and `102` on the way to `10234` are
legitimately not songs, so an error there would be on screen for most of every entry and would teach
the singer to ignore the line. Nothing is drawn for one, and `no song NNNN` stays on submit, where
somebody has actually asked.

**The lookup is best effort and never waits**: installing a package holds the catalog for the whole of
its transaction, and a blocking read on every keystroke would freeze the picture for the length of an
install, so a busy catalog is simply not an answer and the next key press asks again. That is three
outcomes rather than two, and collapsing *missing* into *busy* would either stall the display or
record a miss nobody looked up.

## The name on the idle screen is one word in two colors

**`Karaoke` in the lyric face's near-white and `Machine` in the sung-lyric amber, set touching.** It
is one word and it stays one word — there is no space, no capital-letter gap and no second line; what
changes is that the two halves are colored apart. This is the mark everywhere else: the Android TV
banner sets the name in two colors and the website's `<h1>` does the same thing in HTML. A product
whose wordmark is two-tone on its launcher tile and its home page and plain on its own front screen
has three treatments of one name, which is two too many.

**The second half is the amber, on all four surfaces.** The blue is still the *screen's* accent
everywhere else; what it is not is the wordmark's. The icon is what makes a disagreement visible — it
paints its **M** in the amber and the banner blits that icon in a few millimeters from the name.

**The halves are measured as prefixes, not as two strings.** `measure_line` is given
`["Karaoke", "Machine"]` and `offsets[1]` is where the second half begins with the kerning between `e`
and `M` already in it; measuring the two apart and adding the widths would open a gap exactly the
width of that kern pair, and a gap is the one thing this may not have. It is drawn from a computed
left edge rather than centered twice, because two centered draws would each center themselves and
overlap. This is `examples/banner.rs`'s treatment moved onto the screen, not a second invention of it.

**`TITLE` and `TITLE_HALVES` are one string written twice, and a test says so.** The fit test — the
one that drops the title to the smaller face when `KaraokeMachine` will not span a phone held in
portrait — measures `TITLE`, while what gets drawn is the halves. `the_title_halves_spell_the_title`
is what stops a rename touching one and not the other, which would size the title against a name
nobody sees.

## What the idle screen says about the catalog

**One dim line under the heading: `1,234 songs · 5 packages`, and `No songs installed` when there are
none.** The idle screen answers *where the remote is* and *type a number here*, and the question
somebody standing in front of the machine asks first is whether there is anything in it. A full
catalog and an empty one otherwise look identical, so the first way anybody learns that no package was
ever installed is a number being refused, which is a poor way to find out and gives no hint what to do
instead.

**Zero is stated in words rather than as `0 songs · 0 packages`**, because a row of zeroes reads as a
broken counter and the empty catalog is precisely the state that needs explaining.

**Under the title rather than inside the Remote control panel**, which was the other candidate: that
panel is narrow and already carries a headline, an address, a QR code and up to three lines of detail,
and it is a panel about *reaching* the machine rather than about what is in it. The position is a
constant between two others — `TITLE_TOP`, `CATALOG_SUMMARY_TOP`, `PROMPT_TOP` — and a test computes
all three glyph boxes from the `Theme` rather than trusting the numbers, so changing a font size fails
a test instead of putting two lines through each other on a television nobody is standing in front of.

**Idle only**, like the fault line above it: there is no catalog question while somebody is
singing, and drawing it during a song would be a library read per frame for something nobody can see.
The queue disc in the corner is the counter-example on purpose — that one is on both screens, because
*who is next* is a question during a song where *what is installed* is not.

**The counts are best effort, exactly like the dialled-song preview** — `try_lock`, never `lock`,
because an install holds the library mutex for a whole transaction — and a busy library leaves the
last answer on screen rather than blanking the line, since an install is the one moment the counts are
both changing and unreadable. Counting is skipped entirely unless `catalog_version` moved, and that
comparison happens **inside the same lock** as the counts, so a version can never be paired with
counts from the other side of an install.

The published idle screenshot states the **real** size of the catalog the picture is of, read off the
running machine by `tools/dev/screenshots.sh`: `What the README may show of a catalog` fabricates only
what would leak a machine — the LAN addresses and the queue — and how many songs somebody has leaks
nothing.

## What the connect panel says about the other addresses

**The preferred address in full, and a count of the rest: `1 other address`, `2 other addresses`.**

Listing them assumes the preferred one is a guess. It is not: `km_api::connect` ranks the candidates,
demoting virtual adapters by interface name and preferring the private range a home router actually
hands out. What a list does in practice is spend the panel — which is narrow, and hand-wraps its
detail to at most three lines — on WSL, Hyper-V and Docker addresses that nothing off that computer
can reach, and then truncate the one address it came to show. A truncated URL is worse than a count,
and nobody standing in front of a television reads four of them off it.

**Nothing is dropped, only unshown**: `ConnectInfo::urls` is unchanged, the machine logs
`also reachable at` once per address at startup, `GET /api/v1/discover` returns the whole list, and
the mDNS advert publishes every one — so the operator who genuinely has to pick a different address
has three places to find it, none of them a television.

The count is singular at one, because `1 other addresses` is the kind of thing that makes somebody
doubt the rest of the screen.

## The connect panel's box is the size of what is in it

**The height is the rows added up; only the width is a fraction of the screen.** The idle panel and
the `I` overlay were `0.26h` and `0.18h` of the screen, their rows were `0.55`, `0.70` and `0.85` of
*the box*, and the air around them came from the box's **width** — three rules that were never
compared. The second detail line was drawn a few pixels below the panel's own bottom edge and the
third entirely outside it, so a machine bound to loopback said `Listening on 127.0.0.1:8177.` and
then a half-height smear where the sentence explaining how to fix it should have been. It was
reported from a real run.

The third size already worked this way — the standing card is its QR plus its caption plus air, and
the box is whatever that comes to — so this is the pattern it already had, applied to the two sizes
that carry words. A change to a font size now moves the box instead of overflowing it, and a test
walks every row of both sizes at every screen in `SCREENS`.

**What is drawn is centred in the box, because most states are not the worst case.** The box has to
be the size of the most the panel can say — `PanelSize::rect` answers callers holding no panel, and a
reservation that shrank with the message would be no reservation — but a reachable machine says three
rows where a failed bind says two and neither says three lines of detail. Rows pinned to the top
padding put all of that slack in one lump along the bottom edge, which reads as a panel with a hole in
it; the old fractions spread it by accident, and by drawing outside the box.

**The detail gets the whole panel when there is no QR code**, which is the shape of the problem
rather than a saving: loopback-only, no network and a failed bind are the three states that have a
sentence to read, and they are exactly the three with no address to encode. Reserving a square
for a code none of them draws wrapped the explanation into two thirds of the width, which cost lines
in the one place lines were short — the overlay wrapped the loopback sentence to *four* and dropped
the fourth without saying so.

**A cut is marked with `…`**, as the standing notice and the keypad message already were; one
implementation now, because the argument was written out three times. A `ServerFailed` detail is the
operating system's own words about a socket and nothing bounds their length.

**Two things are pinned that the panel could otherwise eat.** The overlay's code is capped below the
standing card's, or a box grown to hold three lines would have quietly taken the *bigger code in a
smaller card* claim away from the panel whose whole shape rests on it. And every code is capped
against the panel's width: sized from the height alone, the idle panel's QR on a 1080×2400 phone is
384 pixels in a 594-pixel box, which left the text column negative and drew the words across the
code.

**The three rows are a budget and not a list of three detail lines**, which is what lets the key hint
in [`F11 opens the remote in a browser`](#f11-opens-the-remote-in-a-browser) be drawn without the box
moving. A box that grew for it would put a QR code through the words being sung: every size here is
anchored under the lyric band, and the clearance is not the same on every screen — the ultrawide
leaves the standing card nine pixels against a row costing forty-four, where the 16:9 screens leave
about 0.038 of the height. That measurement is a test rather than a paragraph, because it is the
number the next person weighing a fourth row needs.

The one screen this cannot satisfy is that portrait phone, where the small face is sized from a
height more than twice the width and sixteen characters fill a line; there the sentence is cut and
marked, and the test names that screen so a second one joining it is a failure rather than a silence.
It is the same screen the standing panel is given up on.

## The song book

**A PDF of every song, four columns wide, written by a small PDF writer of our own.** The paper half
of a karaoke machine, modeled column for column on a real commercial one — A4 portrait, 233 pages for
11,999 songs, 52 rows a page — with `ARTIST | CODE | TITLE | FIRST LINE`, alphabetical by artist
inside a section per language. Every column already exists: `artist`, the code, `title`, and the
`lyric_preview` from `A song's first lines travel with it`. **In the machine's own locale**, which
`?locale=` overrides for one printing — so the book it copies is Portuguese and this one can be,
which is what `The interface has a locale; a song has a language` in
[`foundations.md`](foundations.md#the-interface-has-a-locale-a-song-has-a-language) buys here.

**Its chrome is cp1252 and always was**, so the ceiling is a fact about the format rather than a new
rule: base-14 fonts, no `/FontFile`, `winansi.rs` transliterating what it can and counting what it
cannot. Portuguese fits inside it; a locale that did not would report replacement characters the
corpus never had, and a test asserts no catalog does.

**Four columns, not five, and the code column carries the whole number.** A code **is** a six-figure
number — `3500` is bank 3, slot 500 — so printing it whole is the only correct answer. **A `VOL`
column holding the bank was rejected** though the reference has one: a reader would have to
concatenate `3` and `005` to dial 3500, which works only if the slot is zero-padded, and a code column
that cannot be read off in one piece is worse than one column narrower. The freed 24.2 pt and its
gutter go half each to the title and first line, the two columns that truncate.

**The table is ruled the way the reference's is**, measured off its own drawing operators: every cell
boxed, headings repeated and boxed on each page, the code centered and the three text columns ranged
left, and the grid **stopping at the last row** rather than ruling empty cells to the margin. Every
page is ruled identically, which is what makes the grid a function of one number — how many rows this
page carries.

**The section is a running header, not a band.** It was a band across all four columns with no
vertical through it, drawn on the page a section *started* on. That is wrong for a book of this shape
and the fault is arithmetic: a section is routinely twenty pages long, so pages two through twenty
said nothing at all about which language they were, and a heading you have turned past is not on the
page you are reading. So the language is set in the **top right of the masthead, on every page** — a
third field on the title's own baseline, beside whose book it is at the left and what the document is
in the middle.

Two things fall out of it and both are wanted. **It costs no body row**: `SLOTS_PER_PAGE` is now
every page's whole allowance rather than 52 minus two on a section's first page, which is worth two
pages in a book of two hundred. And **the subtitle does not repeat it** — `?language=` adds no
clause under the title, because the same fact on the same line twice is noise; the song count and
the catalog version stay, having nowhere else to appear.

**A section still starts a new page**, which is what makes a running header honest: no page holds two
languages, so the one in the corner is true of every row under it.

**Three ways to ask for one, because there are two questions.** `karaokemachine --song-book FILE` and
`GET /api/v1/songs/book.pdf` print an *installed catalog*, so the numbers carry the bank the machine
**assigned**; `km-pack book a.kmpkg b.kmpkg` prints files no machine has seen, so they carry the bank
each package **suggests** — or, where it suggests none, the one its id implies. A machine's owner may
move a package and the book follows; a packager printing volumes they are about to hand out has
nobody's machine to consult.

`km-pack book` warns rather than refusing, in two ways. No suggestion and an unparseable one cannot
happen, since a bank is a `u16` and a package that names none gets one from its id. What remains is
two packages wanting one bank, and two songs landing on one code — the second is a book that *lies*
and `Library::install` makes it impossible, but packages bound for different machines may legitimately
share a bank, so refusing would make the command useless. Capped at ten lines, because a shared bank
makes every song collide.

**A book says whose machine it is, and that is a second string rather than a rename.** The top left
carries a name, set by `--book-name`, `?name=` or `km-pack book --book-name`; the centered `SONG LIST`
above the columns is set only by `km-pack book --title`. They answer different questions — whose
machine the book belongs to, and what the document is — and a house with a machine in two rooms is the
case the first exists for. It is drawn like any other text, so a name outside cp1252 is transliterated
rather than opening a second encoding path.

**And the machine's own name is the default for it, composed rather than substituted.** A machine
called `Living Room` prints `KaraokeMachine - Living Room`; one still called what it was shipped as
prints `KaraokeMachine`. Composing is the point — a book saying only `Living Room` would have dropped
the half that says what kind of list it is — and it is applied where `?name=` is *read* rather than
inside the style builder, which is what makes the `ETag` follow it: the composed string is what
reaches `BookQuery::name_tag`, so renaming the machine changes the validator and a cache cannot serve
the old book. `km-pack book` is unchanged and has no machine to ask.

**We write the PDF rather than taking a crate, and the base-14 fonts are what buy that.** A reader
must supply the glyphs and metrics for `/Helvetica`, so a font object is four keys and no stream — no
font file, no parser, no subsetting, no CID machinery, which is where the bulk of any PDF library
lives. `km-songbook` takes `km-songcode` and **no external dependency at all**, the same judgment
`Rendering CD+G, not converting it` made.

**The price.** Text is WinAnsi, so Latin letters cp1252 lacks are transliterated (`ā` prints as `a`)
and anything else — Cyrillic, Greek, CJK — becomes `?` and is **counted**, with the count reported
by the command line and on the book's first page. That is the standing
`No complex-script text shaping` non-goal met again, and a Japanese-titled song still gets a row
carrying its code and artist. Streams are uncompressed for the dependency reason: `flate2` reaches
this workspace only through `zip`, so `/FlateDecode` would mean a new entry. About 3 MB for twelve
thousand songs, against the reference's 6.7 MB.

**Two things the reference does that this improves on.** A repeated artist is blanked but **reprinted
at the top of every page**, because a page is what somebody reads on its own; and a song with **no**
artist prints an em dash rather than a blank, since a blank already means "the same as above" and
would file an unattributed song under whoever preceded it.

**Ordering lives in `km_songbook::arrange` and the accent folding does not.** The fold has to be
`km_song::text::fold` so the book's alphabet is the search box's, and that crate drags `midly` and an
encoding detector — so the two adapters (`km_api::book`, `km_pack::book`) fold and one `arrange` sorts.
Sections are keyed by the **raw** language code and named through `km_kmpkg::Language`, so `ENGL` and
`PORT` print as themselves rather than merging; `und` heads `Undetermined`, which is a package saying
somebody looked and could not tell.

**The route is public**, on the same argument the catalog export makes and an easier one — a printed
song list is the most public artifact a karaoke machine has. **Its `ETag` is not the bare catalog
version**, because the body varies with `?language=`, `?package=` and `?name=`, and a shared
validator would serve a cached Portuguese book to a request for English. `?name=` is **hashed** into
that header rather than interpolated, and it is the only one that has to be: the other two are
closed sets, while a name is free text and a `"` in one would close the entity tag early.

**The machine's own remote serves it; the offline one links to it.** The reason the offline remote
does not *serve* a book still holds and is not softened: its mirror is a different schema with a
different sort key, so a book built there would be a second adapter for one route. Linking out is a
different act — the machine card's *Open in browser* anchor already sends the machine's own base URL
to the real browser on all five hosts, and this is that field read a second time with
`/api/v1/songs/book.pdf` on the end. No proxy route, no client method, no second adapter.

What that leaves is the sentence the original was really about, and it is now the wrong sentence: *a
phone downloading three megabytes to print from is not a workflow anybody has* was written about a
phone, and the machine card is on **all five hosts** — including a desktop window beside a printer,
which is exactly the device somebody prints from. Gated on having an address rather than on being
online, like the anchor beside it: the banner is already saying that address is being retried.

## A lyric line that will not fit

**The size gives way, per line, and never the words.** A line too wide for the screen is drawn in the
largest of four progressively narrower faces that fits, chosen for each of the two rows separately.

A lyric is centered on `layout.w / 2.0` with its *true measured width*, so anything wider puts its left
edge at a negative x and overhangs both sides while SDL clips them. The corpus holds single lines of
**1,667** characters, so this is not a synthetic case.

**The words may not be abbreviated and may not wrap.** Abbreviating is obvious enough — a singer needs
the whole line — and wrapping is ruled out by the layout rather than by taste: the row beneath the
current line already holds the *upcoming* line, and `lyric_row_height` (0.13) leaves about 0.028h of
slack over `lyric_size` (0.085), so a second row of anything would collide. This is the same answer
`draw_idle` gives for the product's name.

**`km-song` answers a different half of this and does not answer this one.** It breaks a run a file
marked nothing across, at the pauses the file's own timing shows, because such a run is not a line the
file placed — [`A file is trusted for the lines it marks`](songs.md#a-file-is-trusted-for-the-lines-it-marks-and-not-for-the-verse-it-says-nothing-about).
What reaches here is what that leaves: a line a file wrote long on purpose, and a run holding no pause
to cut at, both of which are the file saying this is one line. Second-guessing either would make things
worse, so the size gives way instead.

Three answers, and each is about a different file. The pack in `A downloadable song pack` produces its
lines at the right width at the *source*, which only reaches files this project publishes. `km-song`
recovers a line from a file that stopped speaking. This one is what is left when a file has spoken and
the line is still too wide.

**The ladder is purpose-built rather than reusing `text_size`.** Falling back to the ordinary text
face, as the idle title does, is a 55% drop (0.038 against 0.085), so a line one character too wide
would come out at little over half height; `[0.80, 0.65, 0.50]` means the usual answer is the first
step and the line stays a lyric.

**When even the narrowest will not fit, it is used and SDL clips it.** There is no legible rendering of
a 1,667-character line, and every alternative is only a different way of being unreadable.

## A song whose words are turned off draws none, and says so in the corner

**The lyric band is empty, the way it is for a video song**, and the badge run at the top right
carries `no lyrics` while the song plays. The decision itself is
[`A song's words can be turned off`](songs.md#a-songs-words-can-be-turned-off-and-three-faults-turn-them-off-without-being-asked);
what is here is what a room sees.

**`(no lyrics in this file)` is suppressed with the rows rather than reused.** That sentence is a
true and useful thing to say about a file with no words in it, and a false thing to say about a file
that has some and was told not to show them. It keeps its own meaning, and the badge carries this
one, so the screen never claims a file is empty because somebody made a choice about it.

**Nothing is drawn in the band's place.** A second sentence explaining the feature would be the
machine talking about itself to a room that wants to sing, and the badge has already said the only
word there is to say.

**The badge stands whatever the guide melody is doing**, and it stands first in the run. One is
about sound and the other is about the screen, so a song can legitimately have a guide tune and no
words, and suppressing the melody badge would hide a control that still works. First because the run
is cut to fit against the queue pill: the badge that explains why the middle of the screen is empty
must not be the one the cut takes, where a key or a tempo reports a setting the singer chose and can
hear.

**Key and tempo still appear.** Turning the words off changes nothing about the music, and a MIDI
song with its words withheld transposes and changes tempo exactly as it did.

## What the machine does when nobody is singing

**Demo mode: after a minute of silence the machine starts a random song, and when that song ends it
starts another with no gap at all.** Off by default, on in `settings.json` if the owner wants it always
on, switchable at run time by an admin-only route, and with a public route beside it that starts a
*single* song without turning the mode on — argued in
[`Starting one demo song is anybody's`](api-and-network.md#starting-one-demo-song-is-anybodys-turning-demo-mode-on-is-not).

**A minute, sized against the silence rather than against the interruption.** A room that has
genuinely stopped singing spends a second minute looking at an idle screen, which is the silence this
feature exists to fill; a minute is long enough to be a person thinking and short enough that somebody
who put the phone down finds out the box has a catalog. `demo.delay_secs` is the number, and
`PUT /api/v1/admin/demo/delay` sets it on a machine with no way to edit a file — see
[`The demo delay is a route of its own`](api-and-network.md#the-demo-delay-is-a-route-of-its-own-and-it-is-always-written-down).

**A settings file that already names a delay keeps it, because a changed default is not a new
settings version.** `settings_version` moves for a setting whose *meaning* changed, and a delay
somebody may simply prefer is not that.

A commercial home unit does not sit under a television in silence. The catalog is the product, and a
room full of people otherwise has no way to hear what the box holds without first working out how to
drive it.

**One asymmetry carries the whole feature: a demo song ending
starts the next one immediately, and everything a person does buys the full delay back.** Those look
like two rules and are one. A demo chaining with no gap is what a real machine does — the sound never
drops out, so nobody in the room has to wonder whether it has broken or whether it is waiting for
them. But the moment a person is involved at all, the machine's job is to get out of the way for long
enough that they can decide what happens next; a minute of silence after somebody sings is not the
machine being slow, it is the machine not interrupting. "A person is involved" is deliberately one case
rather than several — a real song ending, a stop, a queue edit, a skip taken off somebody's turn — and
the alternative was a policy table in which every row said the same thing.

**Skipping a demo starts the next demo at once, because it is the demo ending rather than a person
taking a turn.** Whoever pressed it is asking for a different song, and a minute of silence is the
machine answering a question nobody put to it. This is the same test the rest of the feature applies —
*is anybody's turn on the deck?* — and a demo is the one loaded song for which the answer is no, so
skipping one ends no turn and buys nothing back. An empty deck is the other state for which the
answer is no, and [it is answered with a song too](#skip-into-silence-asks-demo-mode-for-a-song).

**Stop is the press that asks for quiet, and it keeps the full delay.** The two are a pair, and
splitting them is what stops the rule above from being a hole: skip asks for the next thing, stop asks
for nothing at all. A machine that answered stop with another song two seconds later would be ignoring
the person who pressed it, which is the reason every other transport command — pause included — stays
on the delay's side of the line.

**A demo skipped with the mode off still stops dead**, and nothing enforces that separately. A
one-shot demo is one song by the same rule that makes it one song when it runs out: chaining is what
`demo.enabled` buys, and the trigger deliberately does not turn it on.

**Queueing a song takes the deck off a demo at once, and this is the asymmetry above reaching one
case further.** A machine singing to itself is filling a silence, so it has no turn to lose, and the
rule that a person's involvement gets the machine out of the way governs here exactly as it governs
the delay. It has no business making somebody press a second button to be heard over it: the room's
instinct is that choosing a song is what makes it play.

**A demo stopping mid-verse has the most visible cause this machine can give.** The objection that
answers a *person's* song being cut — it stops with nothing to explain why — does not reach a song
nobody asked for, ended by the singer's own press a half-second earlier. You queued, and your song
came on.

**A person's song is protected by the same check, and this is not a loosening of that.** Queueing
behind somebody who is singing waits. The machine asks *is anybody's turn on the deck?* by reading
the loaded song's origin, and a demo is the one loaded song for which the answer is no. Skip takes a
turn away and is folded away behind a disclosure on the remote for that reason.

**The ending is published as its own reason, `yielded`**, beside finished, skipped and stopped. A
remote reporting *skipped* here would tell a room that somebody had taken a turn away, when what
happened is that a turn began.

**Because it is loaded rather than queued, both screens have to say so.** Music playing with an empty
queue that nobody started is indistinguishable from a fault, and somebody who assumes the machine is
working through a list will wait for a turn that never comes: the next demo starts the instant this
one ends. The television says `DEMO · QUEUE to sing next` on a row of its own, and the remote says the
longer form, because the remote has room and is where the queueing actually happens. **Both
name the act that gets somebody a turn**, so neither may say *skip*: a screen asking for a press the
machine does not need is worse than a screen saying nothing. The idle screen says nothing about demo
mode, deliberately — it already carries the machine's name, the connect panel, the catalog counts,
notices and the number pad, and "a demo song will start in a while" is not a question anybody
standing in front of it is asking.

**The clock counts idleness, not time since the switch**, which is what makes turning demo mode on from
a phone do something visible. A machine that has been quiet for ten minutes is already past its
deadline and starts a song on the next poll. Arming a fresh delay when the switch is flipped reads
tidier and is worse: somebody turns it on, nothing happens for a minute, and they conclude it does
not work.

**Songs are drawn at random from the whole catalog above a suitability floor, which defaults to 5.** An
unattended machine playing its roughest file is a poor advert, and suitability is exactly the judgment
wanted here — it is what the packager measured about the *file*. The floor falls back to the whole
catalog when nothing clears it, because a silent machine is a worse answer than a mediocre song.

Two consequences fall out of it. A video or MP3+G song carries a flat 10 by what it is rather than
by measurement, so **the floor prefers them** — right for showing the machine off, and worth a
thought on a box that will then decode 1080p for as long as nobody comes home. And the floor drops a
song with *no* rating rather than treating it as average, so a package built by something other than
`km-pack` is simply never demoed.

The last twenty picks are remembered so consecutive songs differ. It is not a guarantee: a catalog
smaller than that plays something recent rather than refusing to play.

## Skip into silence asks demo mode for a song

**A skip with nothing on the deck starts a demo song, and only while the mode is on.** Skip asks for
the next thing, and on a machine that is choosing songs for itself the next thing is the machine's to
pick — so the press is answered by getting on with it rather than by a sentence about an empty deck.
It is the same test the rest of the feature applies, *is anybody's turn on the deck?*, reaching the
one state where there is no deck to read: a demo is the loaded song for which the answer is no, and an
empty deck is no song at all. With the mode off nothing is choosing anything, there is nothing to ask
for, and the refusal stands.

**It is the same one-shot `Play something` sets, and a one-shot rather than a change to the clock.**
The delay still moves the way every person's act moves it, so the silence somebody bought a moment
ago is still theirs if no song can start — which is what keeps
[the rule above](#what-the-machine-does-when-nobody-is-singing) that every other transport command
stays on the delay's side of the line true as written. The four refusals hold too, so a queue still
plays first and a machine off the screen still stays quiet.

**Stop is still the press that asks for quiet**, and the pair is what makes this narrow rather than a
machine that will not take no for an answer. Skip asks for the next thing, into silence as much as
over a song; stop asks for nothing at all, and answering it with a song two seconds later would be
the machine ignoring whoever pressed it.

**This is the press somebody already makes, not a new way in.** `Play something` is the labeled
control, and on a machine whose owner never turned the mode on it is the only one. But whoever is
standing at a machine that has been filling the gaps between songs all evening reaches for `N`, and a
machine that answers *nothing is playing* one poll before it plays something has answered a question
nobody put to it.

**One refusal code and one sentence, whichever way it goes.** `nothing_playing` is what every client
already renders for this press, so nothing has a new failure to learn. The demo's own four reasons
belong to the trigger that exists to give them, and
[the route is public](api-and-network.md#starting-one-demo-song-is-anybodys-turning-demo-mode-on-is-not).

**No on-screen key comes with it.** The idle screen says nothing about demo mode, the number pad is
twelve keys, and the strip is the controls for a song that is playing. Touch has the remote's own
skip, which is the same press arriving from the same rule.

## The remote can ask for a demo song without turning demo mode on

**`Nobody singing? [Play something]` on the Now tab's player card, in both remotes, disabled unless
nothing is loaded and the queue is empty.** The row above gives the machine a delay and an
owner's switch, and between them they leave one case out: a room that wants to hear something *now* and
has not been quiet for a minute, on a box whose owner never turned the mode on. That is most of the
times somebody picks the phone up in a silent house.

**A button on the card that deliberately has no transport, and this is the distinction that keeps that
decision intact.** `The transport is folded away, and only on the Queue tab` took pause, restart,
skip and stop off this card because they end somebody's turn. This cannot: the machine refuses it while
anything is loaded or queued, so at the moment it is pressable there is no turn to end and nobody to
interrupt. It is the only control here that is not an adjustment to a performance, and the only one
enabled precisely when the others are not.

**Disabled rather than absent**, which is the rule at the top of `_player.html` rather than a fresh
judgment: absent is reserved for what a *mode* does not have at all, and this condition flips on every
song start and every queue add. It is the most volatile predicate on the card, and a control coming and
going that often reads as a fault.

**On the player card rather than in the Machine section**, which was the other candidate. That section
exists only in the offline application — online the machine *is* this process and there is nothing to
choose between — so putting it there would hide it from the remote most people open, which is the one
the machine serves. The card already knows both conditions, in both remotes.

**The press answers with a toast, and it is the only control here that needs one.** The machine sets a
flag and its own poll thread starts the song a moment later, so the card re-read to answer the press
still says `Nothing playing` — true when it was taken, and about to stop being. `Starting a song…`
covers the gap until the event stream republishes the card, and covers the case where that republish
arrives before the swap does.

## Demo mode has a key on the machine's own keyboard

**`D` turns demo mode on and off, and turning it on starts a song straight away.** The two rows
above give the mode a clock, an owner's switch behind a password, and a one-shot button
on the remote. Between them they leave out the person actually standing at the machine, who would
otherwise have to pick up a phone and sign in to `/admin/` — on the one surface where nobody is
holding a phone.

**A keyboard is not a guest with the address, which is what makes this consistent with
[`Turning demo mode on is an owner's act`](api-and-network.md#turning-demo-mode-on-is-an-owners-act-knowing-it-is-on-is-not)
rather than an exception to it.** That decision gates the *route* because somebody who found the
machine on the network could otherwise make a box in a house they are not in play music indefinitely.
Whoever presses `D` is in the room the sound comes out of and can press it again. It is the position
`--play`, `F10` and dropping a package on the window all already take about what a person at the
keyboard is allowed to do — and of those, opening a folder is the one that reaches *outside* the
machine, where this does not.

**`D`, a bare letter, because the function row is full and has nothing to pair with.** `F1`…`F9` are
the transport strip, `F10` is the packages folder, `F11` the remote and `F12` the frame meter.
`Ctrl+F11` is free, but the only modified key on that row that is not a debugging control is
`Ctrl+F10`, and its whole argument is the pairing — `F10` shows you where songs go, `Ctrl+F10` picks
up what you put there. Nothing on that row pairs with demo mode, so a `Ctrl+F<n>` would be an
arbitrary number beside an unrelated key. The letters are for whoever is standing at the machine,
which is exactly who this is for — and the same reasoning put `T` beside `F` rather than on a
modifier.

**Turning it on asks for a song rather than moving the deadline.** The clock counts *idleness*, so a
press in a room that has just been singing would do nothing at all for a minute — the switch
would look broken in precisely the situation somebody reaches for it. So the press sets the same
one-shot the remote's `Play something` sets: a press is a person saying *now*, not a deadline
arriving early. It follows that the four refusals apply unchanged — a song loaded, a queue to play
first, no sound, off the screen — and all four mean *not yet* rather than no, because the mode is on
regardless and takes the deck as soon as it clears.

**It does not persist, where the owner's switch may.** The route takes a `persist` flag and this
does not: `settings.json`, `/admin/` and the API remain the ways to say what the machine should do
when nobody is in front of it, and a key pressed at the machine is the evening's switch. A key that
quietly rewrote `settings.json` would be the only binding on this keyboard that did.

**Switching it off leaves the song playing**, for the reason the route gives: it is a mode, not a
transport command. `N` is one key away for somebody who meant stop, and it is the same key that takes
a demo's turn in every other circumstance.

**It answers on the flash band, and it is the only toggle here that needs to.** The rule on this
screen is that a success has the screen itself to speak with — a queued song starts, or shows as
`next:`. A mode has nothing: `DEMO · QUEUE to sing next` is drawn for the length of any demo song
whether the mode is on or off, so the screen cannot say which, and switching it off mid-demo changes
nothing visible whatsoever. The band it uses is the *`Done`* one rather than the number line, which is
failures only and painted `theme.alert` — good news sent through it arrives in the color of bad news,
which is a fault this screen has had once already.

**No on-screen key, and no row on the transport strip.** The idle screen says nothing about demo mode
by an earlier decision and still says nothing; the strip is the controls for a song that is playing,
and it is full.

## The address stands in the corner while the machine sings to itself

**A demo song puts the remote's URL and a QR code on the television for as long as it plays, in a third
panel size that is a code with a caption and nothing else.** It goes when a song somebody queued
starts, and it is the only time the address is on screen without being asked for.

**This is the missing half of demo mode rather than a decoration on it.** The feature exists because a
room should be able to hear what the box holds *without first working out how to drive it* — and the
one thing that would let them drive it is otherwise on the idle screen they are not looking at and
behind `I`, a key nobody in the room knows about. A demo song is precisely the moment when everybody
present has been given a reason to want the remote and no way to get it.

**Four things take it away, and each is already the rule for something else on this screen.** The `I`
overlay wins, because it is bigger, carries the detail lines, and somebody asked for it. The transport
strip wins, for the reason the demo line already yields to it. A number being dialled wins,
because the dial box is half the width and centered, so its right edge reaches exactly this panel's
left edge — the `I` overlay overlaps it too and gets away with it by being gone in eight seconds. And a
queued song takes it away by ending the demo.

**A fifth is the sharpest: the demo line outranks the panel.** `The demo line, and why
it does not share a row` settled that the instruction may not be shortened, and the panel is only ever
up while that line is — so a screen with room for only one of them keeps the sentence. This is measured
rather than asserted: `standing_panel_fits` asks what the line would come out at and the four landscape
screens in `SCREENS` leave it between 84 and 112 characters against a floor of 24.

**The portrait phone is where it bites and where the panel is given up.** The small face is sized from
height, so at 1080×2400 twenty-four characters of it want 69% of the width and there is no corner left
to put a code in; a window that shape is not one anybody watches a demo song on from a sofa, and
dropping the panel there beats a code nobody can scan or an instruction nobody can read.

**The two axes are decided by different things.** The height is the code's, because a QR is a square
and what makes one scannable across a room is how big it is. The width is the *caption's* — sized to
`http://255.255.255.255:65535`, the longest address `km_api::connect` can produce, so the address is
never shortened and the panel never changes width between one machine and the next. Sized on both
axes from the code it comes out 130 pixels wide at 720p and prints `http://19…`, which is worse than
printing nothing: somebody whose camera will not focus is exactly who a caption is for. What the
shape buys is a code **larger** than the `I` overlay's in a card a fifth the area, because the
headline and the three detail lines are gone.

**It says nothing about the key that opens the remote here**, and that is the same shape as the
paragraph below rather than a second decision. The other two sizes carry that line out of rows they
already have; this one has a code, an address and padding, so there is no row to take it from and
nowhere to put another — the ultrawide gives the card nine pixels of clearance under the lyrics
against a row costing forty-four, and the code may not shrink below the overlay's without giving up
the claim the card exists for. Somebody sitting at the machine during a demo song presses `I`, which
is the size that does say it.

**And with no address it draws nothing at all**, where the other two sizes say what is wrong in words.
Loopback-only, no network and a failed bind each have an honest one-line explanation, and a line is the
thing this size has no room for; a card reading *No network* for the length of every demo song, every
demo song, is noise. The idle screen and the `I` overlay both still say it, and they are where somebody
diagnosing it is looking.

## The display draws at the screen's rate, not as fast as it can

**Vsync is requested, and a driver that refuses it is not an error.** SDL3 dropped SDL2's
`SDL_RENDERER_PRESENTVSYNC` creation flag and `sdl3` 0.18.4 wraps no replacement, so this is one FFI
call to `SDL_SetRenderVSync` whose answer is logged beside the renderer's name. Both say the same kind
of thing — what the machine actually got, rather than what it asked for — and "why is this choppy?"
needs both.

Without it the machine draws **118 frames a second into a 60 Hz television**, the compositor discarding
half of them: about a third of a Cortex-A55 spent on pictures nobody sees. Measured on the appliance:
40.5% of one core during a 1080p song before, 28.9% after.

**Plain vsync, not adaptive.** `SDL_RENDERER_VSYNC_ADAPTIVE` tears rather than waits when a frame runs
late. On a machine whose whole job is words over a picture, a torn frame is worse than a repeated one:
somebody is reading it, and a tear lands across the line they are on. Adaptive is also the less widely
supported value, and there is no reason to ask for the exotic one.

**A frame budget stays behind it.** A software renderer or a headless compositor can refuse vsync, and
then nothing else limits the loop; the 8 ms floor keeps it from spinning at four thousand frames a
second.

**The measurement also found something this does not fix**, recorded so the next person does not
mistake this for having addressed it: the **idle** screen costs 95.9% of one core, more than twice what
playing a video does, because a frame takes ~15 ms to build there against ~5.5 ms during a song. Vsync
trims idle by a ninth and no more, since idle was never the case running away with the frame rate. See
[`docs/architecture/video.md`](../architecture/video.md#the-display-draws-at-the-screens-rate-and-idle-costs-more-than-video).

## Rendered text is kept, and the cache is bounded because the alternative already crashed

**A string drawn this frame is very likely the string drawn next frame, so it is kept.** Rasterising
text with SDL_ttf, uploading it to the GPU, drawing it and destroying it on every frame for every
string is most of what the display does: 88% of a core during a MIDI song, against 10.7% once the
textures survive. The words on screen change a few times a minute; without a cache they are rebuilt
sixty times a second.

**The cache holds textures rather than rendered surfaces, and that is a decision rather than a
detail.** Profiling says uploading a texture costs twice what rasterising the glyphs does — 25% of
the app against 13% — so the natural design, caching what SDL_ttf produced, recovers the smaller
third of the waste and looks like a reasonable job.

**It is bounded, at four megapixels, evicting what was least recently wanted.** Not caution in the
abstract: these exact textures leaked once, `GL mtrack` reached 2.6 GB, and Android's low-memory killer
took the app down about ninety seconds into a song. An unbounded cache is that failure with a slower
fuse. Four megapixels is about two screens' worth and measures 70 MB in practice, against a video
song's frame pool which is larger.

**The risk it accepts is font identity.** Entries are keyed on the address of the font that drew them,
because SDL's `Font` exposes no stable handle and two faces of one size cannot otherwise be told apart.
Addresses are only identities while the fonts stay put, and the display rebuilds them on a resize — so
the cache must be cleared there, and the failure if it is not is *wrong glyphs* rather than a crash:
the previous size's text, drawn confidently. The type system cannot enforce it because the display owns
its fonts by value. **A guard that is a call rather than a type is the weak point of this design**, and
and it matters before reusing the cache anywhere else.

The measurements, the profile and the asymmetry that found it are in
[`docs/architecture/video.md`](../architecture/video.md#a-rendered-string-is-kept-not-remade-every-frame).

## The wallpaper folder is chosen again, not once

**The three-candidate choice — the owner's folder, the checkout overlay, the shipped set — is re-made
at every rescan rather than frozen at startup.** And `debug.wallpapers` names extra files shown on top
of whichever won.

**The bug this prevents is invisible and lives in the gap between two true sentences.**
`Where the owner's own wallpapers live` chooses between the three **by contents**, deliberately, so
that an empty folder falls through. `Zipped wallpapers` rescans the folder every cycle, so images can
be dropped in while the machine runs. Both work — but if the *contents* are live and the *choice* is
not, the machine where that matters is **every machine before its first picture**: `Paths::create`
makes the owner's folder empty on purpose so it can be found, so it loses the argument at startup, and
dropping a picture in then rescans a folder that has already lost. Nothing in any log says so. "Drop a
picture in and it appears" is true of the second picture and false of the first.

**It must be able to move back as well as forward**, which is why the obvious shortcut — once the
owner's folder wins, stop asking — is refused. This row's neighbor promises that emptying the folder
returns the shipped set, and a one-way latch would quietly retire that.

**The cost is stated.** The scan is one directory read plus each archive's central directory, and it
happens once per wallpaper change rather than once at startup — thirty seconds apart by default, and
once more each time a song starts under
[`A song starting changes the picture`](#a-song-starting-changes-the-picture), on a machine doing
nothing else, against a handful of entries. Every change restarts the interval, so a song start moves
the next scan rather than adding one; the rate only rises where songs are skipped faster than the
interval, which is nobody singing. The cheaper arrangement, if it is ever
wanted, is to have the contents test hand back the `Playlist` it has already built instead of throwing
it away, which is refused because it would put a display type in a path type's return
value.

**One definition, three readers.** The rule lives in `WallpaperSettings::folder`, and
`Settings::wallpaper_config`, the display loop's rescan and `--show-paths` all ask it.

**`debug.wallpapers` is additive and is not the `dir`-as-a-list fix.** Extra images or `.zip` packs on
top of the folder that won — the same two kinds the folder itself takes, so `Zipped wallpapers` extends
for free. Merging the three candidates is still refused, and a list of *directories* cannot express a
single named image anyway. So it is a second field.

**The trap it creates, and where the guard is.** `holds_wallpapers` — the contents test that decides
which candidate wins — must pass no extras. Counting them would let one debug entry make an empty owner
folder "hold wallpapers", suppressing the shipped set and leaving the screen showing that one picture:
an additive setting turned into a replacing one, which is the black-screen trap
`Where the owner's own wallpapers live` exists to prevent, arriving by a third door. It is `&[]` at
that call site with a comment, and a test asserts it.

**Nothing in `debug.` is the machine's to delete**, here as everywhere.

**And the API reports which rule won.** Four values rather than three and a `null`, because "a setting
named it" is an answer rather than an absence. `WallpaperState.current` deliberately carries a bare
file name — the folder is the operator's filesystem layout and no remote's business — and the *rule's
name* is publishable exactly where the path is not. With the image count and the problem field beside
it, a client can answer the only question an owner actually asks: `bundled` with a count of four means
their folder held nothing when it was last looked at, which is every cycle rather than once.

## Removing a wallpaper

**`DELETE /api/v1/wallpapers/{id}`, and a Remove control on `/admin/pictures`.** The same argument
`Removing a bank` makes: on a box under a television there is no file manager, and on Android the
folder is app-private storage that no USB copy or `adb push` reaches.

**The unit is a pack, not a picture.** A zip is one row saying how many pictures it holds, and
removing it removes all of them. That is the rule `Zipped wallpapers` argues from the other end — a
collection arrives as one file to drop in, so it is one file to remove — and it is what a package
keeps: you uninstall the package, never a song inside it. Taking one entry out of an archive would
mean rewriting a file the machine did not make, on the live scan path, under names somebody else
chose. Since [`A wallpaper is a pack`](#a-wallpaper-is-a-pack) there is no loose image to be the other
kind of row, so the rule is the whole of what this route does rather than one of its two cases.

**Only the owner's own folder yields a removable row**, and the two refusals are permanent ones so a
page can spend them by leaving the control off rather than drawing one that is always denied. A
`debug.wallpapers` entry is not the machine's to delete — `Only \`debug.\` names a file` says what that
section names belongs to the owner — and neither is a `wallpaper.dir` folder, a checkout's overlay, or
the bundled set, which would come back on the next start and so would be a delete that succeeded and
convinced nobody.

**The picture that is showing needs no fallback, where a bank does.** A bank is *named by a
setting*, so deleting the named one has to choose another first or leave the machine pointing at
nothing; a wallpaper is a position in a list rebuilt from the folder, and the playlist re-finds the
showing image by value and falls to the first entry when it has gone. So the file goes first here
where a bank's goes last. What the delete does have to do is ask for the next picture at once — a
picture still on the television after being removed is a broken button — and mark the folder stale,
because removing the last of the owner's pictures hands the rotation back to the shipped set.

**Its own ACL id, `wallpapers.remove`, rather than sharing `wallpapers.upload`.** A bank's delete
shares `audio.write` because that id already means *which banks this machine keeps* and there is one
tab where both happen. Pictures are the other shape: `km-admin` sends them from another box, so "may
this machine be given a picture" and "may this machine's pictures be deleted" are asked by different
people in different rooms.

**A picture id carries its file extension**, where a bank id is slugged from the stem. Two reasons, and
the second is load-bearing: a folder may hold `sunset.jpg` beside `sunset.png`, which a stem would give
one id and make one of them undeletable; and `/wallpapers/next` is a static segment sitting exactly
where `{id}` goes. Axum matches the path before the method, so that literal wins for every verb — a
test pins it — and what keeps them apart is that no id can *be* `next`, because a scanned file always
has an extension.

## What language the television speaks is a machine setting

**`machine.locale`, beside `machine.name`, because it is the same kind of fact** — something the
owner set about this machine rather than about a request. A room has one television and one language;
a page has one reader and follows theirs. See
[`A viewer chooses the remote's language`](remotes.md#a-viewer-chooses-the-remotes-language-and-the-machine-does-not-choose-it-for-them),
which is the other half and deliberately not this one.

**It needs no migration.** Every settings field is optional and the default is the source language, so
an existing install reads as English and nothing about it changes. A tag with no catalog — a typo, or
one written by a later version — falls back to English rather than refusing to start: an appliance
under a television that will not come up is the worse failure by a distance.

**It takes effect on the next frame, not the next start.** `km-display` holds no state, so the locale
rides on every `Frame` and the machine reads it per frame; there is nothing cached to invalidate,
because a catalog is immutable and one exists per locale already.

**A picker sets it, because the alternative is hand-edited JSON.** What language a *page* is in
follows the browser and is changed by whoever is looking at it; what language the *screen in the
room* is in is nobody's browser's business. The heading on that card says which of the two it is,
plainly, because confusing them is the likeliest misreading of the whole feature — and the
confirmation is said in the language just chosen, so somebody who picked the wrong one finds out at
the browser rather than at the television.

**`GET /locale` is public and `PUT /admin/machine/locale` is not**, which is the split
[`The URL prefix is the permission`](api-and-network.md#the-url-prefix-is-the-permission) draws for
`/demo` and `/debug`: how a machine is set up is anybody's business and changing it is the owner's.
The write sits under `/machine/` beside the name because the two are the same kind of fact. The read
is a route of its own rather than a field on `/discover`, which a remote reads before it knows
anything and on every poll: what a screen in another room says changes nothing a phone does.

**Both admin surfaces draw the pane**, the machine's own `/admin/` in process and `km-admin` over
that route. A box that arrives speaking the wrong language is one somebody fixes at the desk they
set it up from, beside the name and the password they are already setting there — and a machine
whose screen nobody can read is the case where reaching it from another room matters most. A tag
this build has no catalog for is a 400 rather than a fallback: `best_match` already reaches `pt-BR`
from `pt` and `pt-PT`, so what is left is a language the machine could not draw, and a 200 would
report a change nobody could see.

**The number separator follows the words.** `12.489 músicas` and `12,489 songs`; a comma beside
`músicas` is the same inconsistency a full stop beside `songs` would be, and the two only ever move
together.

## Every program says which build it is

**One version number for the whole repository was already decided; this is where each program shows
it.** [`One version number for the whole repository`](repository.md#one-version-number-for-the-whole-repository)
makes the number, `tools/dev/check-version-pin.sh` keeps the two workspaces' copies equal, and until
now the only place it reached a person was the `/discover` payload — so four programs could be four
builds and nothing on any screen would say so. The comparison somebody actually wants is between two
surfaces: this tool against that machine, when one of them has just done something surprising.

**Each shows it where that program already keeps facts about itself.** Once, except on the machine,
which is the only one of the four whose hosts do not all give it a window to write on.

- **`km-admin` and `km-package-builder`** put it last in the header's right-hand cluster, after the
  tag naming the machine they are pointed at. Deliberately **not** styled as a chip like that tag: a
  chip there is a link to where you change the thing it names, and a version is not a control. It is
  the same dim weight as the counts and the folder path beside it.
- **The remote** has no header — a phone gets the bottom tab bar and nothing else — so it goes at the
  foot of Setup, which is already the tab about this program rather than about a song. One line, no
  label, outside the setting groups.
- **The machine says it in the window title and in the idle screen's bottom-left corner.**
  `KaraokeMachine 1.8.0` is what the taskbar, the dock and the alt-tab switcher say while the machine
  is idle, and it shares that title with the song, behind it, under
  [`The window title says what is playing`](#the-window-title-says-what-is-playing). **The window is
  not enough**: an Android television and the appliance on a bare TTY draw no window furniture at
  all, so on the two hosts a room actually contains there is nowhere but the screen. The corner
  carries it small and dim, in the face and color the catalog summary above it uses, with a leading
  `v` — no label fits beside it, and that letter is what makes a bare number read as a version.
- **The idle screen only, and the number pad gives up a line for it.** A song is the show, and a
  version number over somebody singing is furniture nobody in the room can act on. The pad is
  bottom-left as well and nothing fits above it, so its grid sits one small line higher and a screen
  too small for both gets no pad — which is what that pad does with anything it cannot fit.

**The remote reports the version of whatever is serving it**, which is the offline app's in one mode
and the machine's in the other. Both are true statements about the process answering the request, and
which is what makes the line an answer rather than a constant printed twice.

**It is not a catalog string.** A product name and a number are the same in every language, and a
translator handed `KaraokeMachine 1.8.0` can only make it wrong. Every surface that has a catalog
keeps this out of it; what a version's tooltip says around it is a key like any other.

**What is still hand-copied is the mobile shells' own version** — `ports/*/android/app/build.gradle`
and `ports/remote/ios/project.yml` — which `check-version-pin.sh` does not read, because it reads
Cargo manifests. Those name the *package* to an app store rather than the build to a person, and the
line inside the remote is served by Rust either way.

## The window title says what is playing

**The one fact about this machine an operating system will draw for a window nobody is looking at.**
The screen is the show and the show is across a room, so the operator working at the box has the
machine behind a browser: the package builder is on one window, the owner's page on another, and the
question they have is *what is up now?*. A taskbar button, a dock tooltip and an alt-tab entry all
answer it without a click, and none of them can be reached from the television.

**`Exagerado — Cazuza — KaraokeMachine 1.8.0`, in that order.** Title before artist, which is the
order [`QueueEntry::label`](../../crates/machine/km-queue/src/queue.rs) and the queue overlay already
put a song in, so the three surfaces naming one song name it one way. The product and its version go
last because every one of those three places truncates from the right at a width it chose: the song
is what a glance is asking for, and the version is what somebody goes looking for deliberately and
can widen a window to read. An idle machine says the bare
[`KaraokeMachine 1.8.0`](#every-program-says-which-build-it-is)
and nothing else, so nothing has to be trimmed to find it.

**A song with no performer draws no empty separator**, and an artist recorded as blank counts as
none — the package builder's *Title from file name* writes exactly that, so it is not a shape only a
malformed file can reach.

**It is pushed from the frame loop, guarded on the two names rather than on the composed string.**
The window belongs to that loop and no handle leaves it, so there is nowhere else it could be pushed
from. `SDL_SetWindowTitle` is a round trip to the window manager, and composing a title to compare
would allocate sixty times a second to learn that nothing moved — so what is kept is the title and
artist last drawn, and the frames between one song and the next do no work at all.

## The machine can be shut down from in front of it and from a phone

**Until this, the only way to turn the appliance off was at the wall.** That is not the same as
being wrong — the machine writes the catalog, the password, the session epoch, the machine name and
the demo state at the moment each changes, *because* a shutdown hook is what a power cut does not
run — but it does lose whatever `persist()` was holding, and it asks somebody to do the one thing
every appliance in the house tells them not to.

There are now four doors and two destinations. **A single press of the box's own power button turns
it off cleanly**, with no confirmation on the television, because the person pressing it is standing
at the machine and a set that asks *are you sure* from across a room is worse than one that does
what it was told. The owner's page carries the same act behind the admin password, beside a
**Restart** that ends the application and lets the supervisor start it again. And `Ctrl+Q` is the
keyboard's spelling of whichever of the two the host can do.

**`Ctrl+Q` and not `Q`.** Plain `Q` shows the queue, is documented in the README, and is the kind of
binding somebody learns without being told — so taking it would mean the letter people press to
glance at what is next ends the evening instead. The modifier is what this project already uses:
`km-tray` puts `⌘/Ctrl+Q` on the Quit item of the two programs that have a menu, and the machine now
agrees with them. This is a fourth door onto one exit in exactly the sense
[`The windowed tools have a menu bar on macOS`](#the-windowed-tools-have-a-menu-bar-on-macos)
already argues for the third.

**On the appliance `Ctrl+Q` powers the box off rather than quitting**, and that is not two different
bindings. On a computer, quit means the application closes and the desktop is still there. On a box
under a television there is no desktop to return to and `Restart=always` brings the machine back two
seconds later — so quitting would show a console briefly and then the machine again, a quit that
visibly does not quit. Turning the box off is what somebody at that keyboard meant.

Which of the two it is, is a **runtime capability and not a build**: there is no compile-time notion
of "appliance" on Linux, and gating on one would leave a developer's `cargo run` able to switch their
own desktop off. See
[`Power is a capability of the host, not a method on the machine`](api-and-network.md#power-is-a-capability-of-the-host-not-a-method-on-the-machine).

**Shutting down asks first and restarting does not.** That extends the rule in
`ConfirmPage` rather than breaking it. What earned the three delete confirmations was not that they
touch a file — it was that nothing on the page undoes them, and shutting the machine down has that
property more strongly than any of the three: bringing it back needs somebody to walk to the box. A
restart interrupts the evening for ten seconds and mends itself, which puts it beside *show the next
picture*.

**Both answer a page rather than the redirect every other control on that page ends with.** A
redirect after a restart is a guaranteed failed load, because the machine is down for the seconds the
browser spends following it; after a shutdown it is a spinner and then a connection error. Both are
truthful and both read as the button having broken. The restart page brings the tab back by itself
with a plain `<meta http-equiv="refresh">`; the shutdown page does not, because a page that kept
retrying a box which is off would show a connection error all evening.

**The card is absent on a machine that has no power control, not disabled**, which is
[`A control that can only be refused is left out, not grayed`](api-and-network.md#a-control-that-can-only-be-refused-is-left-out-not-grayed)
applied to a capability rather than to a state. A grayed control invites somebody to work out what
would enable it, and the answer here — *run this on a supervised appliance* — is not something anybody
can do from that page.

**What is deliberately not offered: rebooting the box.** *Restart* means the application, which is
the thing an owner actually needs after changing a setting that is only read when the machine starts.
A box that needs rebooting needs somebody at it, and that person has a power button.
