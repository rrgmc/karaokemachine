# What the owner's `/admin/` pages say.
#
# The source catalog: every message is written here first, in US English, and every other locale is
# a translation of this file. See `The prose and the names are US English`.
#
# **A different reader from the singer's remote.** This is whoever set the machine up, sitting down
# to it on purpose — so a sentence here may explain a consequence, where a toast on a phone may not.

## The pages

admin-title = Setting up the karaoke machine
tab-machine = This machine
tab-songs = Songs
tab-sound = Sound
tab-pictures = Pictures

## Signing in

sign-in = Sign in
password-field = The password
password-placeholder = at least 4 characters

## Logging a tool in to the machine

login-yes = This program is logged in.
login-no = This program is not logged in, so nothing here can be changed yet.
login-needed = This machine asks for its password before anything can change. Type it here.
login-forgotten = This computer has forgotten that password.

## The machine

machine-name = Machine name
machine-where = Where to find it
machine-unreachable = Machine not reachable.
machine-open-hint = { $addresses ->
    [one] Open this on a phone to search and queue songs.
   *[other] Open one of these on a phone to search and queue songs.
 }
password-change = Change
password-change-warning = This machine has a password. Changing it signs out every browser, including this one.

## Songs

songs-add = Add songs
songs-empty = No songs yet. Add a package above.
column-package = Package
column-songs = Songs
column-numbers = Numbers
column-bank = Bank
column-move = Move
package-bank = Package { $package } bank
package-remove = Remove { $package }

## Sound

bank-add = Add a sound bank
bank-add-hint = A SoundFont decides what the instruments sound like. An .sf2 file, up to 1 GB.
banks-here = Banks on this machine
bank-use = Use this one
bank-bundled = came with the machine
bank-playing = playing
bank-remove = Remove { $bank }
column-file = File
column-size = Size
package-flag-uncurated = uncurated
column-version = Version
output-heading = Where the sound comes out
output-hint = The machine remembers this.
output-system = Follow the system
output-choose = Choose the output
output-save = Use this output
output-playing = Playing through { $device }.
output-is-system = what the system points at now
output-absent = not plugged in
output-changed = The sound comes out of { $device } now.
output-changed-system = The sound follows the system default now.
output-fell-back = The output that was chosen is not there, so “{ $device }” is playing instead.
output-show-all = Show every way of naming these
output-show-fewer = Show one row per output
output-busy = The output cannot be changed while a song is playing or waiting. Stop the music first.
output-none = This machine did not report any outputs.
level-heading = How loud the machine sends it
level-hint = Set this once, then use the amplifier’s own volume. Sending at full is usually right.
level-now = Sending at { $db } dB.
level-decibels = { $db } dB
level-choose = Choose the level
level-save = Use this level
level-change = Change the level
level-hide = Leave the level alone
level-changed = The machine sends at { $db } dB now.
level-deeper = The control goes quieter than the slider does.
level-none = This output has no level the machine can set. Whatever it is plugged into holds the volume.
level-unreadable = That level could not be read.
level-confirm-from = Now
level-confirm-to = After
confirm-level-heading = Turn the machine up?
confirm-level = This is the level going into the amplifier, and everything will be louder by the same amount. Turn the amplifier down first if the room is already set up.
confirm-level-button = Turn it up

## Pictures

picture-add = Add a pack
picture-showing = Showing now
picture-next = Next
picture-changes-every = Changes every
picture-interval = { $seconds ->
    [one] { $seconds } second
   *[other] { $seconds } seconds
 }
picture-interval-and-song = { $interval }, and when a song starts
picture-in-folder = In folder
picture-remove = Remove { $picture }

## Actions

action-add = Add
upload-sending = Sending. Leave this page open until it finishes.
action-remove = Remove
action-rename = Rename
action-cancel = Cancel
action-on-screen = On screen
action-on-machine = On this machine

## Language

# The picker by which the owner sets what the *television* says.
#
# **The only language control on these pages, and it is not about these pages.** What language
# `/admin/` itself is in follows the browser and the `km_locale` cookie, which on the machine the
# singer's remote writes at `/` on the same origin — and on `km-admin` its own front door writes,
# from a key in that program's catalog. What the *screen in the room* says has no other way to be
# set but by hand in `settings.json`. So the heading says which one it is, plainly, because the
# likeliest misunderstanding of this whole feature is that the two are one thing.
locale-machine = Screen language

# The way back to a host's front door, which only a host that can be pointed at more than one
# machine draws: in the tab strip, and beside the sentence saying whether this program is logged in.
# What changes either -- an address box, a password field -- is on that door and not here.
machine-elsewhere = Different machine

# The two doors out to a host's own searching. Only a host that serves those pages draws them.
pictures-find-heading = Find some
pictures-find-hint = Search for photographs, check that words will read over them, and build a set to send.
pictures-find-link = Find pictures
banks-fetch-heading = Get one
banks-fetch-hint = Download a sound bank and send it, for a machine with no internet of its own.
banks-fetch-link = Get a sound bank
locale-save = Save
locale-changed = The screen is now in this language.
no-such-locale = This build does not have that language.

## Demo mode
#
# The two boxes are two different questions and the words have to keep them apart: one is about
# tonight and the other is about every night after it. The four confirmations say which of the two
# just happened, because "Saved." would leave somebody unsure whether they had changed the machine
# for an evening or for good.
#
# The hint no longer names the delay, because the box below it does. A sentence and a box saying the
# same number is one of them going stale the moment somebody edits the other.
demo-heading = Demo mode
demo-hint = When nobody is singing, the machine plays a song it chooses, and then another.
demo-enabled = Enable demo mode
demo-persist = Keep this after a restart
demo-save = Save
demo-on-run = Demo mode enabled until restart.
demo-on-stored = Demo mode enabled, including after a restart.
demo-off-run = Demo mode disabled until restart.
demo-off-stored = Demo mode disabled, including after a restart.
demo-delay-label = Start after this many seconds of quiet
demo-delay-save = Save
demo-delay-saved = The machine will wait { $seconds } seconds. Saved for good, as a delay always is.
demo-delay-bad = That is not a number of seconds.

## Power
#
# The card is absent altogether on a machine whose host has no power control, so none of these are
# ever shown beside a button that cannot work.
#
# The two are deliberately not parallel in weight. Restarting interrupts the evening for ten seconds
# and mends itself; shutting down ends it and needs somebody to walk to the box, which is why only
# one of them asks first.

power-heading = Power
power-restart = Restart the application
power-restart-hint = Some settings only take effect when the machine starts. This restarts it without touching the box.
power-shutdown = Shut the machine down
power-shutdown-hint = Do this before unplugging it, so nothing is lost.

confirm-shutdown-heading = Shut this machine down?
confirm-shutdown = The machine goes off. Somebody has to press its power button to bring it back.
confirm-shutdown-button = Shut it down

farewell-restart-heading = Restarting
farewell-restart = The machine is starting again. This page comes back on its own in a few seconds.
farewell-shutdown-heading = Shutting down
farewell-shutdown = The machine is going off. Press its power button when you want it again.

## Confirming a removal
#
# These pages exist because the thing they do cannot be undone. They name what goes, in a sentence,
# before the button that does it — so the prose is deliberately longer than anything on the singer's
# remote, and every one of them says what does *not* come back.

confirm-remove-heading = Remove “{ $name }”?
confirm-remove-button = Remove it
confirm-remove-package = Its { $songs ->
    [one] one song goes
   *[other] { $songs } songs go
 } from this machine, and the package file itself is deleted.
confirm-remove-bank = The bank file is deleted from this machine.
confirm-remove-bank-playing = This is the bank the machine is playing.
confirm-remove-picture = The picture file is deleted from this machine.

## What a page says when it cannot find what a URL named

no-such-package = There is no package by that name.
no-such-bank = There is no bank by that name.
no-such-picture = There is no picture by that name.
confirm-remove-picture-last = This is the last custom picture. The machine will use the default ones.
pictures-in-it = Pictures in it

## Prose the pages carry
#
# Longer than anything on the singer's remote, deliberately: this reader is deciding whether to do
# something, and the consequence is what they need before the button rather than after it.

password-new = New password
machine-name-hint = Machine name. Up to 63 characters.
machine-name-field = Name
pictures-bundled-hint = Default pictures. Adding your own replaces them.
picture-kinds-hint = A zip of pictures. JPEG, PNG and WebP inside it.
picture-showing-nothing = nothing yet
songs-add-hint = A package is one file carrying its own songs, numbers, titles and artists. Build one with the Package Builder, then send it here.
songs-count = { $songs ->
    [one] { $songs } song
   *[other] { $songs } songs
 } in { $packages ->
    [one] { $packages } package
   *[other] { $packages } packages
 }.
page-title = { $machine } — setup

## The Problems tab
#
# The one tab that is always in the bar whether or not it has anything to say, so its empty states
# are load-bearing rather than decoration: a page that says nothing reads as broken where a page that
# says everything is fine reads as a machine that is fine.

tab-problems = Problems
problems-refused-heading = Packages failing
problems-refused-hint = These package files are on this machine, but their songs could not be loaded.
problems-refused-empty = Every package was loaded.
problems-other-heading = Everything else
problems-other-empty = The sound and the pictures are working.
column-what-is-wrong = What is wrong
problems-delete = Delete the file
problems-delete-label = Delete { $file }
problems-go-there = More about this on its own tab
problems-choose-bank = Choose a sound bank
confirm-delete-heading = Delete “{ $file }”?
confirm-delete-button = Delete the file
problems-nothing-refusing = This fault no longer occurs. It has been fixed, or the file is gone.
problems-file-gone = The file is gone.

## Where the wallpapers on screen come from

pictures-from-owner = Your own pictures.
pictures-from-overlay = Local pictures.
pictures-from-bundled = Default pictures.
column-folder = Folder
confirm-delete-problem = The file is deleted from this machine.
confirm-delete-problem-rebuild = If you can build this package again, rebuilding and sending it fixes the problem rather than only removing the file.
pictures-from-setting = Pictures are coming from the folder set by wallpaper.dir in settings.json, which takes priority. Pictures added here will not be shown until that setting is removed.
factory-password-banner = This machine is still on the default password.
factory-password-change = Change it
factory-password-explained = This is the password the machine generated at its first start, and it is shown on the machine's screen. Once you change it, the screen stops showing it.
password-reset = Reset to a new PIN
sessions-heading = Signed-in devices
sessions-explained = Signs out every phone and browser, including this one, without changing the password.
sessions-reset = Sign out everywhere
access-heading = Who may do what
access-explained = A phone with no code gets the room level. A code raises a phone to its level. The password above opens everything.
access-room = With no code, anybody can
access-view = Watch only
access-queue = Queue songs
access-control = Queue, skip and play now
access-queue-code = Code to queue songs
access-control-code = Code to queue, skip and play now
access-code-set = set
access-code-unset = not set
access-code-clear = Clear
access-save = Save
debug-heading = Debugging
debug-explained = Lets the machine play a file directly from its own disk, and enables the debug settings that name individual files. Takes effect at the next restart.
debug-turn-on = Turn debugging on
debug-turn-off = Turn debugging off
switch-after-restart = This is set, and takes effect the next time the machine starts.
console-heading = Development console
console-explained = A page for whoever is working on this machine, at /dev/. Its own copy of the machine's controls needs no password, so anyone on this network can change anything while it is on. Needs debugging on as well. Takes effect at the next restart.
console-needs-debugging = The console is on, but debugging is off. It needs both.
console-turn-on = Turn the console on
console-turn-off = Turn the console off
performance-heading = Frame statistics
performance-explained = Draws what the machine measures about its own screen, over the picture. The same panel the F12 key draws, for a machine that has no keyboard. Takes effect immediately.
performance-turn-on = Draw the panel
performance-turn-off = Hide the panel
songs-uncounted = could not be counted

# What went wrong reaching the machine, for the host that has to reach one over a network.
#
# **Keyed by a code rather than by a sentence**, which is the arrangement `km-remote-pages` already
# keeps: the program that finds out about the fault may be talking to a machine in another language,
# and the page doing the rendering is the one that knows what its reader speaks. A refusal the
# machine worded itself is not in here at all -- `AdminError::Refused` carries it through untouched,
# because the machine is the authority on what a name may be and a second opinion here would be a
# second thing to keep in agreement with it.
#
# These three are listed in `machine::ERROR_KEYS` as well, and a test asserts the two agree -- they
# are a `match` rather than `msg("…")` calls, so `words`' scanner cannot see them.
error-offline = This machine is not answering.
error-unauthorized = This machine requires a password.
error-not-found = Not found.
