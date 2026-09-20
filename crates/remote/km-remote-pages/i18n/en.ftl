# What the singer's remote says.
#
# The source catalog: every message is written here first, in US English, and every other locale is
# a translation of this file. See `The prose and the names are US English`.
#
# **This is read on a phone, at a party, by somebody holding a microphone.** Short and plain beats
# complete and careful — a toast is on screen for four seconds.

## Refusals
#
# **Keyed by the machine's stable code, not by its sentence.** A code this build does not know falls
# back to `error-unavailable`, which is the sentence every one of these had before any of them had a
# name — so an offline remote talking to a newer machine says something true rather than an English
# sentence inside a translated page. See `RemoteError::Unavailable`.

error-unavailable = The machine cannot do that right now.
error-queue-full = The queue is full.
error-unauthorized = This machine requires a password.
browse-back = Back to songs
error-not-found = Not found.
error-not-acknowledged = Sent, but the machine did not confirm it.
error-failed = That did not work. Try again.

# The kind of song is an argument rather than part of the sentence, because English writes `a video
# song` and Portuguese writes `uma música em vídeo` — an article that agrees with the noun. A
# sentence composed by the machine could not be translated without taking it apart again.
error-no-key = { $kind ->
    [midi] This song has no key to change.
    [video] A video song has no key to change.
    [cdg] An MP3+G song has no key to change.
    [ultrastar] An UltraStar song has no key to change.
   *[other] This song has no key to change.
 }
error-no-tempo = { $kind ->
    [video] A video song has no tempo to change.
    [cdg] An MP3+G song has no tempo to change.
    [ultrastar] An UltraStar song has no tempo to change.
   *[other] This song has no tempo to change.
 }
error-no-melody = { $kind ->
    [video] A video song has no guide melody.
    [cdg] An MP3+G song has no guide melody.
    [ultrastar] An UltraStar song has no guide melody.
   *[other] This song has no guide melody.
 }
error-no-melody-channel = The guide melody could not be found in this song.

error-nothing-playing = Nothing is playing.
error-nothing-loaded = No song is loaded.
error-nothing-queued = Nothing is queued. Find a song and add it first.
error-no-sound = This machine is not making sound. Ask the administrator.

error-no-favorites = This remote does not keep favorites.
error-no-demo = This machine cannot start a demo song.

## The tab bar
#
# Three tabs and there is no fourth — see `layout.html`. These are read at a glance, under an icon,
# on a phone: one word each.

tab-songs = Songs
tab-now = Now
tab-queue = Queue
tab-setup = Setup
tab-packages = Packages

## Page titles


## The songs list

search-clear = Clear search
filter-initial = First initial
filter-language = Language
filter-all = All
filter-tags = Tags
filter-tag-add = Add tag
filter-tag-remove = Remove tag
filter-tags-clear = Clear all
load-more = Load more
book-link = The song book, as a PDF
youtube-link = Find on YouTube
no-lyric-match = not found in this song

## A song's actions
#
# Every one of these is an icon button. The label is what a screen reader says and what a long press
# reveals, so it names the *act* rather than the button.

song-queue = Add to queue
song-play-now = Play now
song-play-next = Play next
song-favorite = Add to favorites
song-favorited = In a favorites folder
song-unfavorite = Remove from this favorite folder
song-more-folders = Add to multiple favorites…
row-actions = Show the controls for each song
row-actions-extra = Show the extra row controls

## The queue

queue-move-up = Move up
queue-move-down = Move down
queue-remove = Remove
queue-empty-hint = No song queued. Tap
singing-as = Singing as
singer-name = Name your songs are queued under
singer-placeholder = your name

## Now playing

transport-show = Show controls
transport-play = Play
transport-pause = Pause
transport-stop = Stop
transport-skip = Skip to the next song
transport-restart = Start this song again
control-key = Key
control-key-up = Raise key
control-key-down = Lower key
control-tempo = Tempo
control-faster = Faster
control-slower = Slower
control-music = Music
control-volume = Music volume
control-melody = Guide melody
nobody-singing = Demo song
play-something = Start
demo-hint = Demo — queue a song and it starts straight away
pinned = pinned
reset = reset

## Favorites

folder-new = New folder…
folder-new-name = New folder name
folder-add = Add
folder-done = Done
folder-close = Close

## The machine

tab-machine = Machine
tab-book = Book
machine-none = No machine selected
machine-address = The machine's address
machine-address-example = 192.168.1.5
machine-save = Save
machine-use = Select
machine-use-instead = Select this instead
machine-change = Change…
machine-rescan = Rescan
machine-also-on-network = Also on the network:
machine-open-browser = Open in browser
machine-open-browser-title = open this machine's internal remote in web browser
connected = Connected to the karaoke machine
not-connected = Not connected
reconnecting = Reconnecting…
went-wrong = Something went wrong.



## Language

# The control by which a viewer changes what language this remote is in. Its own label is the one
# string that has to be legible to somebody who cannot read the rest of the page, which is why the
# languages inside it are named in themselves rather than translated — see `Locale::endonym`.
language-picker = Language

## The browse modes

# The three buttons above the search box, and what the box asks for under each. Keys returned by
# `Mode::label_key` and `Mode::placeholder_key` rather than written into the markup, because a
# `match` in Rust is where the three arms already live.
#
# Deliberately separate from `tab-songs`: the tab and the mode button say the same word today and
# are not the same control, and a translator may want to tell them apart.

mode-songs = Songs
mode-artists = Artists
# A glyph, not a word. Do not translate it — it is the same star drawn on every row.
mode-favorites = ★
search-placeholder-songs = Song or number
search-placeholder-artists = Artist
search-placeholder-favorites = Folder

## What the list says about itself

# `112 shown` where everything matching is on screen, `50 of 112` where the total was cheap enough
# to count. Composed in Rust — two numbers in a sentence is arithmetic, and arithmetic in markup is
# where it stops being testable.

list-count-shown = { $count } shown
list-count-of = { $showing } of { $total }

# What an empty list says, and it says *which* filter emptied it: "Nothing matches" on its own tells
# somebody nothing they can act on. Three kinds of list times four reasons, which is a table.

empty-songs = No songs. Is a package installed?
empty-search = Nothing matches “{ $query }”.
empty-initial = No song starts with { $initial }.
empty-initial-search = Nothing starting with { $initial } matches “{ $query }”.
empty-folder = { $folder } is empty.
empty-folder-initial = Nothing in { $folder } starts with { $initial }.
empty-folder-search = Nothing in { $folder } matches “{ $query }”.
empty-artists = No artists. Is a package installed?
empty-artists-search = No artist matches “{ $query }”.
empty-artists-initial = No artist starts with { $initial }.
empty-artists-initial-search = No artist starting with { $initial } matches “{ $query }”.
empty-folders = No favorites yet. Tap a song's ☆ to create your first folder.
empty-folders-search = No folder matches “{ $query }”.

## What is playing

# What the card and the bar say where no song is loaded. A queue with somebody in it is about to
# start, so it says so rather than saying nothing is playing to a person who has just queued a song.

now-nothing-playing = Nothing playing
now-up-next = Up next
# Reads as `for Ana` — the name follows it in the markup, so a language that puts the name first
# has nowhere to say so. Worth knowing before a third language arrives.
now-singer-for = for

## The machine card, and the strip above it

# How this device came to be talking to the machine it is talking to. **Keyed by a stable code**,
# the way a refusal is: `km-remote-core` sends `remembered`, this file says what that means in the
# reader's language. See `words::how_key`.

machine-how-asked-for = asked for
machine-how-remembered = remembered
machine-how-moved = same machine, new address
machine-how-adopted = found on the network
machine-how-chosen = chosen

machine-connected = Connected
machine-rescan-button = Rescan
machine-refresh = Refresh song list
machine-pin-hint = A machine you enter here is remembered even while it is switched off. Rescan to search the network again.
banner-unreachable = The karaoke machine is not reachable.
# Reads as `— retrying 192.168.1.5`, with the address after it.
banner-retrying = retrying

# How much of the machine's catalog this device holds. Composed in Rust, which is what makes the
# machine card the one pushed fragment that is *built* per language rather than only rendered per
# language — see `handlers::everywhere`.
machine-songs-copied =
    { $count ->
        [one] { $count } song in this copy
       *[other] { $count } songs in this copy
    }

## Counting songs

# Under an artist and under a folder, and the same message in both because it is the same sentence.
count-songs =
    { $count ->
        [one] { $count } song
       *[other] { $count } songs
    }

## Odds and ends

# The guide-melody switch. Two words, and they belong to the switch rather than to the melody, so a
# language that inflects can leave them uninflected.
toggle-on = On
toggle-off = Off

song-unfavorite-confirm = Remove “{ $title }” from this folder?

## What a row's button leaves behind

# Two words at most, and they are read at a glance on a row somebody has just pressed.

badge-queued = queued
badge-next-up = next up
badge-playing = playing
badge-sent = sent

## What a song action says

queued-song = Queued: { $title }
playing-next-song = Playing next: { $title }
playing-now-song = Playing now: { $title }
queued-not-moved = Queued { $title }, but it could not be moved up.
next-not-started = { $title } is next, but it could not be started.
demo-starting = Starting a song…

## Choosing a machine

machine-not-chooseable = This remote cannot change which machine it uses.
machine-type-address = Enter an address first.
machine-offer-gone = That machine is no longer available.
machine-found-named = Found { $name } at { $url }.
machine-found = Found { $url }.
machine-scan-nothing = No machines answered on the network. Still using the same address.
machine-scan-already = The machine found on the network is the one you are already using.
machine-scan-kept = Still using the same machine.

# Appended to whichever of the four sentences above a rescan produced, so that a house with three
# machines is told there are two more to look at rather than being told about one.
machine-more-answered =
    { $count ->
        [one] One more available.
       *[other] { $count } more available.
    }

## Your name and your folders

singer-set = Your songs will be queued as { $name }.
singer-cleared = Your songs will be queued with no name.
packages-shown = Packages in my song list
packages-shown-hint = An unticked package stays out of searches, artists and filters on this phone. A song number and your favorites still find its songs.
packages-hidden-saved = Your song list leaves out the unticked packages.
packages-all-shown = Your song list shows every package.
empty-hidden-packages = Packages you hid under Setup, Packages are not searched.
folder-needs-name = Enter a name.
folder-made = Created { $folder }.
folder-renamed = Renamed to { $folder }.
folder-deleted = Folder deleted.
favorite-added = Added to { $folder }.
favorite-removed = Removed from { $folder }.
favorite-removed-short = Removed.

## What this device says about itself

# Keyed by `km-remote-core`'s stable codes — see `words::code_key`. That crate is in this process,
# which is what made it look safe to compose these there, and it has no catalog and no viewer to
# ask, which is what made it wrong.

offline-not-answering = The karaoke machine is not answering.
offline-none-found = No karaoke machine has been found yet.
offline-stream-closed = The karaoke machine closed the connection.
connection-looking = Looking for a karaoke machine…
connection-connecting = Connecting…
folder-name-taken = There is already a folder with that name.
folder-only-one = This is the only folder. Rename it instead.

## Sharing a folder, and carrying the collection to a file
#
# **Five refusals for a code and five for a file, and each names a different remedy.** A camera that
# read three quarters of a code and a file somebody picked by mistake are not the same problem, and
# "that did not work" would be the same useless sentence for both. See `share::ShareError` and
# `backup::BackupError`, which carry the mapping beside the enum.

share-error-format = That is not a favorites code.
share-error-damaged = That code is damaged. Scan or copy it again.
share-error-empty = There are no songs in this folder to share.
share-error-too-large = This folder is too large to share as one code. Copy the text instead.

backup-error-not-ours = That is not a favorites file.
backup-error-damaged = That file could not be read.
backup-error-empty = There are no songs in that file.
backup-error-too-large = That file is too large to be a favorites backup.
backup-error-no-file = No file was chosen.

# The header every share and backup screen wears. These flows run several screens deep, so the way
# out to the folder list is on all of them, beside the step-back that only some have.
step-back = Back
step-exit = Back to favorites

# The two entry links. Sharing is offered only inside a folder, because being in one is what answers
# *which folder?*; a backup is the whole collection and nothing narrows it, so it is offered on the
# Setup tab instead.
share-link = Share
backup-link = Backup
backup-setup-sub = Save your favorites to a file, or restore from one

# The owner's door, on the machine's own remote only. The words are the machine's -- `admin-title` in
# `km-admin-pages` reads "Setting up the karaoke machine", and `Two admin surfaces, one vocabulary`
# gives the machine's spelling the tie-break -- and the sub-line says the password is wanted, because
# a link that only ever ends in a prompt should say so before the tap rather than after it.
owner-page-link = Set this machine up
owner-page-sub = Songs, pictures, sound and its name. Needs the machine's password

# Which device this is. Settled first, because a page that never says it is the failure.
share-send = Send
share-send-sub = Show a code for the other device to read
share-receive = Receive
share-title = Share { $folder }
share-receive-sub = Read a code and add its songs to { $folder }
share-one-way =
    Songs are only ever added — nothing is removed on either device, and reading the same code
    twice changes nothing. This carries { $folder } in one direction, so do it both ways to have
    both devices match.

# Showing a code.
share-code-hint = On the other device, open the same folder, choose Receive, and point it at this code.
share-cant-scan = Can't scan it?
share-code-copy = Copy this and paste it into Receive on the other device.
share-too-dense = Too many songs for one code. Copy the text below instead.
share-code-alt = Code for { $folder }

# Reading one. The five camera sentences are read off `data-` attributes by `scan.js`, because a
# static file cannot go through this catalog — see the head of that file.
share-camera-start = Start the camera
share-camera-stop = Stop the camera
share-camera-starting = Starting the camera…
share-camera-got-it = Code read…
share-camera-refused = The camera was not allowed. Turn it on for this app in the system settings, or use “Can't scan it?” below.
share-camera-none = No camera was found. Use “Can't scan it?” below.
share-camera-failed = The camera could not be started. Use “Can't scan it?” below.
share-point-at = Point this at the code shown on the other device. Its songs go into { $folder }.
share-paste-hint = Paste the code shown under the other device's code.
share-continue = Continue

# What was read, before anything is written. The code itself is never on screen here.
share-confirm-add = Add to { $folder }
share-confirm-mismatch = This code came from { $from }, and you are adding it to { $into }.
share-scan-another = Scan something else

# What a merge or a restore did, in the terms the person watching cares about.
favorites-added =
    { $added ->
        [one] { $added } song added
       *[other] { $added } songs added
    }
favorites-already-here =
    { $count ->
        [one] { $count } already here
       *[other] { $count } already here
    }
favorites-left-out =
    { $count ->
        [one] One song is not in this device's song list, so it was left out.
       *[other] { $count } songs are not in this device's song list, so they were left out.
    }
# Said under a folder rather than after a restore, so it counts what is not on screen instead of
# what was not written. The songs are still in the folder and come back with the machine that has
# them, which is why nothing here says lost or left out.
favorites-not-here =
    { $count ->
        [one] One song in this folder is not on this machine.
       *[other] { $count } songs in this folder are not on this machine.
    }
# The two sentences below say which songs were left out and why, where the one above says only how
# many. Separate because the remedy differs: a package can be installed, a recording that is in no
# package here cannot be conjured.
favorites-missing-package =
    { $count ->
        [one] One song comes from a song pack this device does not have: { $codes }.
       *[other] { $count } songs come from song packs this device does not have: { $codes }.
    }
favorites-missing-recording =
    { $count ->
        [one] One song is not in any song pack here: { $codes }.
       *[other] { $count } songs are not in any song pack here: { $codes }.
    }
count-folders =
    { $count ->
        [one] { $count } folder
       *[other] { $count } folders
    }
share-open-folder = Open { $folder }
share-return-leg =
    This device now has them. To have both match, show this folder's code from here and receive it
    on the other one.
share-show-code = Show this folder's code

# Carrying the whole collection to a file.
backup-title = Backup
backup-save = Save a backup
backup-save-sub = { $songs } in { $folders }, as one file
backup-nothing = There are no favorites here yet, so there is nothing to save.
backup-restore = Restore from a file
backup-restore-sub = Add the contents of a saved backup to your favorites
backup-add-only =
    Restoring only ever adds. Folders that are missing are created, songs that are already here stay
    as they are, and nothing is removed. So restoring the same file twice changes nothing the
    second time.
backup-restore-title = Restore favorites
backup-choose-hint =
    Choose a file saved by Save a backup — on this device, or in Files, Dropbox or Drive. Everything
    in it is added to what is already here; nothing is removed, and folders that are missing are
    created.
backup-paste = Paste it instead
backup-paste-hint = Open the backup file, copy everything in it, and paste it here.
backup-restore-button = Restore
backup-read-from = { $read } read from { $folders }
backup-folders-created =
    { $count ->
        [one] { $count } folder created
       *[other] { $count } folders created
    }
backup-unreadable =
    { $count ->
        [one] One line in the file was not a song number, so it was skipped.
       *[other] { $count } lines in the file were not song numbers, so they were skipped.
    }
backup-format-newer = A newer version wrote that file. Some of it could not be read.
backup-unchanged =
    Nothing was removed, and the file is unchanged — restoring it again would add nothing.
backup-open-favorites = Open favorites

# What a move or a refresh did. The first is prefixed to one of the other three by `handlers`, and
# only where the address actually changed.
machine-now-using = Now using { $url }.
machine-copy-current =
    { $count ->
        [one] Already up to date — { $count } song.
       *[other] Already up to date — { $count } songs.
    }
machine-copy-imported =
    { $count ->
        [one] Copied { $count } song from the machine.
       *[other] Copied { $count } songs from the machine.
    }
machine-copy-not-answering = It is not answering yet. It will be used as soon as it responds.
