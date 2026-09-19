# KaraokeMachine Package Builder's pages, in English.
#
# **Plain application language**, which is the rule this file is headed with because every catalog in
# the project is: `What a user reads is written in plain application language`. The reader here is a
# curator working a corpus of hundreds of thousands of files at a desktop, so the register is the
# operator's: a sentence or two, consequence first, and a word for a control rather than an
# explanation of it.
#
# **Reasoning goes beside the markup, never on the page.** A `{# #}` in the template is where a
# control's argument lives; what is here is what a curator reads.
#
# **`locale` is this interface's language and `language` is a song's.** Both words are in use on the
# same page here — the browse bar filters by the language a song is sung in, and a package carries a
# default one — so a key says `settings-locale-` for the first and `songs-language-` for the second,
# and neither word crosses. See `The interface has a locale; a song has a language`.
#
# **The data stays as it arrives.** A song's title, its artist, a folder's path, a tag somebody
# typed, and the English names `km_kmpkg::Language` carries for the language pickers: those are
# values, and translating them is not this file's job.

## The header ----------------------------------------------------------------
#
# Drawn on every page. The counts are composed in Rust because each is a plural over a number, and
# `km_locale::filters` keeps markup to a key and nothing else.

nav-songs = Songs
nav-lyrics = Lyrics
nav-folders = Folders
nav-favorites = Favorites
nav-duplicates = Duplicates
nav-packages = Packages
nav-scan = Scan
nav-settings = Settings

nav-lyrics-title = find a song by a line of its words
nav-root-title = open a different folder

header-songs = { $count ->
    [one] { $count } song
   *[other] { $count } songs
  }
header-files = { $count ->
    [one] { $count } file
   *[other] { $count } files
  }
header-failed = { $count ->
    [one] { $count } failed
   *[other] { $count } failed
  }
header-favorites = { $count ->
    [one] { $count } favorite
   *[other] { $count } favorites
  }

# The tag naming the machine Play and Install would reach. The address is always in the tooltip, so
# a name never hides where it is.
header-machine-title = { $address } — change it on Settings
header-machine-none = no machine set
header-machine-none-title = Play and Install have nowhere to go until this is set
header-version-title = this program's version

header-open-browser = Open in browser
header-open-browser-title = open this page in your usual web browser
header-quit = Quit
header-quit-title = stop the tool

## Words a control says on more than one page ----------------------------------

action-save = Save
action-cancel = Cancel
action-ok = Ok

## Acting on a whole filter ----------------------------------------------------
#
# The five confirmations, which all say the same three things: how many, which songs, and what would
# happen to them. The count and the button are composed in Rust, each language choosing its own
# plural.

confirm-songs = { $count ->
    [one] { $count } song
   *[other] { $count } songs
  }
confirm-files = { $count ->
    [one] { $count } file
   *[other] { $count } files
  }
confirm-whole-corpus = the whole corpus, because no filter is set
confirm-whole-corpus-unnarrowed = the whole corpus, because no filter is narrowing it
confirm-matching = matching
confirm-with = with
confirm-without = without
confirm-into = into
confirm-out-of = out of
confirm-read-again = read again
confirm-deleting = thrown away
confirm-undeleting = brought back
confirm-delete-packaged = { $count ->
    [one] { $count } of them is in a package
   *[other] { $count } of them are in a package
  }

confirm-set = Yes, set { $count }
confirm-tag = Yes, tag { $count }
confirm-untag = Yes, remove from { $count }
confirm-file = Yes, file { $count }
confirm-unfile = Yes, take out { $count }
confirm-reread = Yes, re-read { $count }
confirm-delete = Yes, throw away { $count }
confirm-undelete = Yes, bring back { $count }
package-add-room-left = which has room for { $room }, so the first { $room } in this order go in

## Filters somebody named -------------------------------------------------------

saved-band = saved
saved-none = Nothing saved yet — narrow the list, then name it.
saved-name-placeholder = name this filter
saved-keep-page = keep the page
saved-whole-corpus = the whole corpus
saved-update-title = make { $name } mean the filter on screen now
saved-rename-title = rename this one
saved-forget-title = forget this one
saved-forget-confirm = Forget the saved filter “{ $name }”? No song is touched.
saved-already-saved = is already saved, as
saved-replace-it = Yes, replace it

## Paging the browse list --------------------------------------------------------
#
# Two sentences rather than one with a mark in front of the total: while a scan is writing rows the
# total climbs between one click and the next, and a reader is better told than left to decode a `~`.

songs-range = page { $page } of { $pages } ({ $total ->
    [one] { $total } song
   *[other] { $total } songs
  })
songs-range-scanning = page { $page } of about { $pages } (about { $total ->
    [one] { $total } song
   *[other] { $total } songs
  })
pager-scanning = scanning
pager-scanning-title = a scan is still writing rows.
pager-first-title = the first page
pager-last-title = the last page

## A song's words ----------------------------------------------------------------

lyrics-decoded-as = decoded as
lyrics-pin-encoding = Pin this encoding
lyrics-none = This file has no lyrics.

## Opening a folder ---------------------------------------------------------------

open-top-title = the top
open-this-computer = this computer
open-this-folder = Open this folder
open-create-database = Create a database here
open-folders-named = folders named
open-part-of-a-name = part of a name

## The filter bar's chips ------------------------------------------------------
#
# What is narrowing the list, each with the link that takes it back off.

chips-showing = showing
chips-remove-title = stop narrowing by this
chips-clear-all = clear all

## A song's tags ----------------------------------------------------------------

song-tags-none = None.
song-tag-remove-title = take this tag off

## A song's text meta events ---------------------------------------------------
#
# The raw tab, which is a table of what the file holds. The three column heads are abbreviations a
# sequencer's own screens use.

raw-none = No text meta events.
raw-track = Trk
raw-tick = Tick
raw-kind = Kind
raw-text = Text

## Paging -----------------------------------------------------------------------

pager-previous = previous
pager-next = next
hits-range = page { $page } of { $pages } ({ $total ->
    [one] { $total } song
   *[other] { $total } songs
  }), best match first

## A package's two buttons ------------------------------------------------------

install-button = Install it into the current machine
install-build-first = Build it first.
install-sends = Sends the built package to the karaoke app.
renumber-button = Re-flow every number from { $start }
renumber-note = Keeps the current order. Any number you set by hand is overwritten.

## Finding a machine on the network ---------------------------------------------
#
# The lead and the tail sit either side of the setting's name, which is drawn as a word a machine
# reads rather than as part of the sentence.

discovered-none-lead = No machines found on this network. A machine is only found while
discovered-none-tail = is on.
discovered-select = Select
discovered-already-set = already set

## Settings: this tool's language ---------------------------------------------
#
# **The heading has to settle which of two languages it means**, because both are on the screen: the
# browse bar filters by the language a song is *sung* in, and a package carries a default one. That
# confusion is the likeliest misreading of this panel, so the heading names the pages and the note
# says the other half outright.

settings-locale-heading = This tool's language
settings-locale-field = Show these pages in
settings-locale-note = The language of this tool's own pages. It changes nothing about the songs: a song's language is what it is sung in, and you set that on a song or on a package.
settings-locale-save = Save

## What the browser says for itself -------------------------------------------
#
# `static/ui.js` says these, and a static file cannot go through the `|t` filter — so the layout
# composes them here and puts them on `<body>` as `data-js-` attributes, which is how
# `km-remote-pages`' `scan.js` already reads its own. Two carry a value only the browser has: which
# request failed, and how. Those arrive with `{what}` and `{status}` standing in for them, so the
# word order is this file's and the filling-in is one string replace in the browser.

js-answered = { $what } answered { $status }
js-unreachable = Could not reach the package builder. Has it stopped?
js-timed-out = { $what } took too long and was given up on.
js-swap-failed = Could not draw the reply from { $what }.
js-the-tool = the tool

## Files that did not parse -----------------------------------------------------
#
# The Reason column's sentences are reached by a key `ScanStatus::key` returns, because a tally is
# counted where no language is in reach.

failures-heading = Files that did not parse
failures-count = Count
failures-reason = Reason
failures-example = Example
failures-remove = Remove
failures-restore = Restore
failures-remove-title = { $count ->
    [one] Accepts the { $count } file failing this way now. A file that starts failing this way later is listed again.
   *[other] Accepts the { $count } files failing this way now. A file that starts failing this way later is listed again.
  }
failures-reasons-removed = { $count ->
    [one] { $count } reason removed
   *[other] { $count } reasons removed
  }
failures-still-here = These files are still here and still did not parse. Removing a reason only takes it off the list above.

failure-parsed = parsed
failure-unreadable = could not be read
failure-not-midi = not a readable MIDI file
failure-not-video = not a readable video file
failure-video-unsupported = a video, and this build has no `video` feature to read it with
failure-missing-graphics = an MP3 with no .cdg beside it, so it has no words
failure-orphan-graphics = a .cdg with no audio beside it
failure-not-audio = not a readable audio file
failure-bad-graphics = a .cdg that draws no words at all
failure-bad-ultrastar = an UltraStar file this machine does not play
failure-ultrastar-audio = an UltraStar file whose MP3 is not beside it
failure-panicked = the parser panicked

## The checklist a long job draws ---------------------------------------------------
#
# Both the Scan page and the Open page list their steps, so these three belong to neither. The names
# of the steps themselves are `scan-phase-` and `opening-step-`, one set per job.

steps = Steps
step-not-needed = not needed this time
step-not-reached = not reached

## Reading the corpus -------------------------------------------------------------
#
# The phases are keys because a phase is named on the worker thread, where no language is in reach.
# The counts are one sentence for the same reason a plural is: seven numbers in a row is arithmetic.

scan-reads-lead = Reads and parses every supported file under
scan-unchanged-skipped = Files are skipped when nothing about them has changed and this version of the program would work them out the same way.
scan-reanalyze = Re-analyze everything
scan-reanalyze-note = reads them all again, and finishes by re-grouping the files that look like one recording.
scan-changed = Scan changed files
scan-last-run = last run
scan-never-run = never run in this folder
scan-stale-analysis = { $count ->
    [one] { $count } song was worked out by an earlier version of this program. Scanning brings it up to date; nothing else needs doing.
   *[other] { $count } songs were worked out by an earlier version of this program. Scanning brings them up to date; nothing else needs doing.
  }
scan-browse-what-was-found = Browse what was found
scan-stopped-early = Stopped before finishing. Everything read so far was saved, but the folder was only partly scanned and is not marked as scanned. Run the scan again to continue: unchanged files are skipped, so it resumes where it stopped.
scan-stopped-in-tail = Stopped after every file was read and saved, so the folder is marked as scanned. The folder list is rebuilt the next time the Folders page opens, which can take some minutes on a large folder.

scan-tally = { $done } of { $total } read · { $percent }% in the database ({ $written } written this run) · { $parsed } analyzed · { $skipped } unchanged
scan-failed = { $count ->
    [one] { $count } failed
   *[other] { $count } failed
  }
scan-waiting = { $count ->
    [one] { $count } waiting to be written
   *[other] { $count } waiting to be written
  }
scan-waiting-title = read and analyzed, not yet committed. The readers run ahead of the single writer thread, so browsing will keep finding new songs until this reaches zero.

scan-found = { $count ->
    [one] { $count } file found so far
   *[other] { $count } files found so far
  }
scan-rate = { $rate } files a second
scan-remaining = about { $time } left
scan-step-if-changed = only if something changed
scan-stop = Stop
scan-stop-title = Stops after writing the files already read. Everything read so far is kept, and the next scan carries on from there.
scan-stopping = Stopping: writing the files already read.

scan-meter-reading = reading
scan-phase-preparing = loading what the last scan found
scan-phase-looking = looking for files
scan-phase-reading = reading and analyzing
scan-phase-indexing = indexing folders
scan-phase-forgetting = forgetting files that are gone
scan-phase-duplicates = looking for near-duplicates
scan-phase-measuring = measuring the corpus for the query planner
scan-phase-stopped = stopped
scan-phase-finished = finished

## The browse table's columns ------------------------------------------------------
#
# `column-language` is the language a song is *sung* in. What language these pages are written in is
# `settings-locale-`, and the two never share a word.

songs-no-matches = No matches.
songs-select-all-title = select all on this page; shift-click two boxes to tick the rows between them
column-artist = Artist
column-title = Title
column-language = Lang
column-length = Len
column-suitability = Suitability
column-score = Score
column-score-title = Your score
column-melody = Melody
column-melody-title = Ticked when a melody channel was detected; hover it for the channel
column-copies = Copies
column-copies-title = Byte-identical copies on disk
column-versions = Versions
column-versions-title = Files that look like the same recording, saved differently

## Walking the folders on this computer ---------------------------------------------

open-indexed = indexed
open-open = Open
open-nothing-found = nothing found
open-no-folder-called-that = no folder here is called that
folders-range = page { $page } of { $pages } ({ $total ->
    [one] { $total } folder
   *[other] { $total } folders
  })

## Favorites ----------------------------------------------------------------------

favorites-new-placeholder = new favorite
favorites-create = Create
favorites-none-yet = None yet.
favorites-name = Name
favorites-songs = Songs
favorites-second-copies = Second copies
favorites-second-copies-title = Entries that are a second version of a song already in this list
favorites-working-list = Working list
favorites-working-list-title = A list that is scaffolding for a later pass rather than a filing
favorites-set-aside-title = the songs in this list have been set aside, not filed
favorites-rename = Rename
favorites-tidy = Tidy
favorites-delete = Delete
favorites-tidy-confirm = { $count ->
    [one] Drop { $count } entry from { $name }? It is a second file of a song this list keeps; the best copy it holds of each stays.
   *[other] Drop { $count } entries from { $name }? Each is a second file of a song this list keeps; the best copy it holds of each stays.
  }
favorites-delete-confirm = Delete { $name }? The songs themselves are not touched.
favorites-delete-confirm-sourcing = { $count ->
    [one] { $packages } holds what this list holds, and its next sync would take out every song this list put there.
   *[other] These packages hold what this list holds, and their next sync would take out every song it put there: { $packages }.
  }

## Searching the words --------------------------------------------------------------

lyrics-contain = lyrics contain
lyrics-part-of-a-lyric = part of a lyric
lyrics-how-it-matches = Accents are ignored. The last word matches as a prefix. Quotation marks ask for the words in that order.

## The two curation tabs ------------------------------------------------------------

curate-package = Package
curate-add = Add
curate-no-packages = No packages yet.
curate-put-ticked-in = put ticked songs in
curate-file = File
curate-no-favorites = No favorites yet.

## Building a package -------------------------------------------------------------
#
# The phase is a value rather than a key, because four of the eight carry a count, a position or a
# file name. See `build::Phase`.

build-phase-starting = starting
build-phase-reading = reading the package
build-phase-packaging = { $count ->
    [one] packaging { $count } song
   *[other] packaging { $count } songs
  }
build-phase-song = song { $index } of { $total } — { $name }
build-phase-encoding = re-encoding song { $number } — this one takes a while
build-phase-writing = writing { $name } — no more songs to read
build-phase-done = done
build-phase-stopped = stopped

build-done = { $done } of { $total } ({ $percent }%)
build-volume-now = Volume { $number }
build-written = { $count } in
build-skipped = { $count } left out
build-encoding = re-encoding this one, { $percent }% — the song count does not move meanwhile

## Searching the words, continued ---------------------------------------------------

lyrics-type-a-line = Type a line and the songs that sing it appear here.
lyrics-none-indexed-lead = No lyrics are indexed yet. Run a scan with “re-read every file” ticked on the
lyrics-none-indexed-tail = page.
lyrics-not-found = Not found.

## Songs with a similar name ---------------------------------------------------------

similar-heading = Songs with a similar name
similar-how-it-matches = Capitals, accents, punctuation and words like “the” or “karaoke” are ignored. A misspelt word still matches. The likeliest names come first, and the last ones may be other songs.
similar-type-a-name = Type a title or an artist and the songs with a similar name appear here.
similar-not-found = No song has a similar name.
similar-likeness-title = how alike the two names are
similar-searched-from-title = the song this search started from

## Songs that sing the same words ------------------------------------------------------

words-heading = Songs that sing the same words
words-how-it-matches = Credits, addresses and section labels are left out, and capitals, accents and punctuation are ignored. A file missing a verse, or one line heard differently, still matches. A cover with reworked words and a medley do not.
words-too-few = This song has too few sung words to find another by.
words-not-found = No other song sings these words.
words-likeness-title = how much of the two lyrics is word for word the same
column-likeness = Likeness
column-language-title = What language it is sung in, as an ISO 639-1 code
column-suitability-title = Automatic suitability, 0-10
column-score-own-title = Your own rating, 0-10 or unset

## The folder tree -------------------------------------------------------------------

folders-all = all folders
folders-none-lead = No folders to show. Either this folder contains no readable songs, or the corpus has not been
folders-none-link = scanned
folders-none-tail = yet.
folders-folder = Folder
folders-songs-title = Distinct songs anywhere beneath it, not files
folders-up = up
folders-files-here = files here
folders-browse = Browse
folders-only-this = Only this folder

## A row in the browse list ---------------------------------------------------------
#
# Four of these carry a value and are composed in `SongRow::say`, where a test can reach them.

row-artist-title = every song by { $artist }
row-melody-title = channel { $channel }
row-versions-title = { $count ->
    [one] this row stands for { $count } file that looks like one recording
   *[other] this row stands for { $count } files that look like one recording
  }
row-hidden-version-count-title = { $count ->
    [one] one of { $count } file that looks like one recording
   *[other] one of { $count } files that look like one recording
  }
row-hidden-version = hidden
row-hidden-version-title = the song list shows another version of this recording instead; open it
row-favorites-close = close the chooser
row-favorites-filed = { $count ->
    [one] in { $count } favorite — choose another, or take it out of one
   *[other] in { $count } favorites — choose another, or take it out of one
  }
row-favorites-working = { $count ->
    [one] in { $count } working list and filed in none — choose a favorite, or take it out of one
   *[other] in { $count } working lists and filed in none — choose a favorite, or take it out of one
  }
row-favorites-none = put it in a favorite
row-take-out-of = take it out of { $name }
row-put-in = put it in { $name }

row-from-file-name = file name
row-no-title-title = no title detected in file
row-edited = ed
row-edited-title = edited
row-deleted = thrown away
row-deleted-title = somebody threw this song away; it is in no list, no search and no package
row-file-name-title = file name
row-language-unknown = unknown
row-language-more = more
row-set = Set
row-score-title = your own rating
row-edit-title = edit the title and artist here
row-youtube-title = search YouTube
row-similar-title = find songs with a similar name
row-words-title = find songs that sing the same words
row-which-favorite = in which favorite?
row-working-lists = working lists
row-no-favorites = None yet — name one and it is made and filled in one go.
row-create-and-file = Create & file

## Opening a folder, while it takes a while -------------------------------------------
#
# The phases are a value rather than a key, because three carry a count and an open runs before any
# page exists. The console banner asks for the English rendering, a log being written in English.

opening-database = opening the database
opening-closing-previous = closing the folder that was open
opening-up-to-date = bringing the database up to date
opening-indexing = { $missing ->
    [one] building { $missing } missing index and gathering statistics
   *[other] building { $missing } missing indexes and gathering statistics
  }
opening-working-out-language = working out what language each song is in
opening-tidying-text = tidying the text read out of the files
opening-folding = folding { $done } of { $total } titles for the browse order
opening-reading-words = reading the words of { $done } of { $total } songs
opening-gathering-statistics = gathering statistics over the whole corpus
opening-folding-journal = folding the journal back into the database
opening-finishing = finishing

# The same eleven again, as the names on the checklist beside the sentence above. Bare of the counts,
# which arrive next to the running rung as `opening-step-count`, and in the present tense of a list
# rather than of a thing happening now.
opening-step-closing-previous = closing the folder that was open
opening-step-database = opening the database
opening-step-up-to-date = bringing the database up to date
opening-step-indexing = building the indexes
opening-step-folding = folding titles for the browse order
opening-step-working-out-language = working out what language each song is in
opening-step-reading-words = reading the words of each song
opening-step-tidying-text = tidying the text read out of the files
opening-step-gathering-statistics = gathering statistics over the whole corpus
opening-step-folding-journal = folding the journal back into the database
opening-step-finishing = finishing
opening-step-count = { $done } of { $total } · { $percent }%

## The folder picker ----------------------------------------------------------------

open-title = Open a folder
open-back = back
open-opening-a-folder = opening a folder
open-choose-a-folder = choose a folder of karaoke files
open-opening = Opening
open-opened = Opened
open-progress = Opening { $root } — { $phase } · { $seconds }s.
open-migrating-hint = A large corpus migrates the first time it is opened by a new build, which can take minutes. It resumes if you stop it.
open-recent = Recent
open-database-gone = its curation database is gone
open-not-found = not found
open-forget-title = take it off this list
open-browse = Browse
open-browse-this-computer = Browse this computer
open-or-type-a-path = Or type a path
open-path-placeholder = a folder of .mid, .kar, .mp3+.cdg or video files
open-create-here = Create here
open-recent-counts = { $songs ->
    [one] { $songs } song
   *[other] { $songs } songs
  } · { $files ->
    [one] { $files } file
   *[other] { $files } files
  }

## Getting into a machine ------------------------------------------------------------

access-signed-in = Signed in to this machine.
access-forget-it = Forget it
access-debugging-off = Turn Debugging off
access-debugging-on = Turn Debugging on
access-what-signing-in-buys = Sending and installing a package require admin access, which this sign-in provides. Debugging is a separate setting on the machine, and the Play button needs it to audition a song on a machine other than this computer. Changing it takes effect when that machine restarts, so the button above shows the setting the machine is currently running with.
access-password-saved = This computer has this machine's password.
access-retype = Type a different password
access-password = Password
access-password-placeholder = admin password
access-remember-it = remember it on this computer
access-sign-in = Sign in
access-which-password-lead = The machine’s admin password, the same one its
access-which-password-tail = page asks for. If no password has been set, use the six-digit PIN shown on the machine’s screen. The password is exchanged for a token held in memory until this tool closes.
access-tick-to-save = Tick the box to save the password in this computer’s config folder, under this machine’s id. It is never saved into the curated folder.
access-owner-only = The file is readable only by you.
access-no-protection = The file has no protection beyond your profile directory.
access-nothing-to-save-for = No machine has answered at this address yet, so there is nothing to save a password for. Press “Save and check” first.

## Files that look like one recording ---------------------------------------------------

duplicates-heading = Files that look like one recording
duplicates-none-lead = Nothing grouped yet. The pass compares every song's shape against every other's and needs a scan to have read them first; on a corpus of a few hundred thousand files it takes a few seconds.
duplicates-none-tail = ends with it, so pressing this is for the times in between.
duplicates-found = { $groups ->
    [one] { $groups } group of files looks like one recording, and the song list shows the best copy of each — hiding { $hidden } songs that would otherwise be curated twice.
   *[other] { $groups } groups of files look like one recording, and the song list shows the best copy of each — hiding { $hidden } songs that would otherwise be curated twice.
  }
duplicates-nothing-to-judge = Nothing was merged and nothing needs judging. Which file of a group is the same recording is answered by listening, so the group is listed on each song's own page with a play button beside every version, and “every version” on the song list's filter bar turns the grouping off.
duplicates-look-again = Look again
duplicates-look-again-note = Reads the whole corpus, and its answer replaces the last one — a pair that no longer matches is dropped. A pair dismissed as different stays dismissed.
duplicates-identical-heading = Byte-identical copies
duplicates-identical-note = Nothing to decide here, and nothing to run. A song's identity is the hash of its bytes, so identical files are already one song with several paths, and the Copies column carries how many.
duplicates-more-than-one-copy = Songs with more than one copy, most first

## Packages -------------------------------------------------------------------------

packages-new = New package
packages-name = Name
packages-name-placeholder = Classic Rock Vol 1
packages-volumes = { $count ->
    [one] { $count } volume
   *[other] { $count } volumes
  }
packages-version = Version
packages-version-title = Three numbers with dots between them, like 1.0.0
packages-publisher = Publisher
packages-first-number = First number
packages-numbering = The package id is generated and never changes, so rebuilding with more songs produces the same package. A song's number inside a package runs 1 to { $highest }, and the machine adds the thousands that say which package it is, so nothing typed here can collide with a package already installed.
packages-create = Create
packages-open = Open package
packages-path-to = Path to a
packages-import-note = Songs are matched to this corpus by content hash, and any corrections saved in the package are imported too. Entries whose files are not under the corpus root are listed rather than skipped silently.
packages-import = Import
packages-from = From
packages-last-built = Last built
packages-never = never
packages-delete-confirm = Delete the package { $id }? The songs and any .kmpkg already written are not touched.

## One package -----------------------------------------------------------------------

package-id-title = This package's generated id. It never changes.
package-id = id
package-volume-format = volume name
package-volume-format-title = How a volume's number is written after the package name in its file and manifest. It is written once the package has two volumes, or from the first when the box below is ticked.
package-volume-format-note = {"{"}n{"}"} is the number: vol{"{"}n{"}"} names them vol1, vol2.
package-number-one-volume = number the volume even while there is only one
package-number-one-volume-title = For a package that will pass 999 songs. Its first file is named vol1 from the first build, so the name stays the same when a second volume starts.
package-find-songs = Find songs to add
package-folder = folder
package-file = package file
package-build = Build the package file
package-build-all = Build every volume
package-build-all-note = Each volume is written into the folder under its own default name.
package-write-listing = also write a song list beside it
package-spec-file = spec file
package-write-spec = Write spec
package-spec-for = Spec for
package-name = name
package-version = version
package-volume = volume
package-publisher = publisher
package-first-number = first number
package-default-language-title = fills any song in this package that names no language of its own. It changes the package, never the songs.
package-unclassified-are = unclassified songs are
package-refuse-to-build = refuse to build them
package-in-this-corpus = in this corpus
package-every-language = every language
package-empty = Empty.
package-member-count = { $count } of { $highest } songs. A package sourced from favorites starts another volume when its lists outgrow this.
package-volume-tab = { $count ->
    [one] Volume { $number } · { $count } song
   *[other] Volume { $number } · { $count } songs
  }
package-number = No.
package-source-missing = source missing
package-numbering-note = Changing a number saves when the field loses focus. A number already used by another song in this package is rejected; numbers are not swapped automatically.

## Sourcing a package from favorites ------------------------------------------

package-sourced-from = Sourced from
package-no-sources-yet = No list yet, so this is an ordinary package.
package-no-favorites-lead = No favorites yet. Make one on the
package-every-list-is-a-source = Every list this package can read is already a source of it.
package-also-source-from = Also source it from
package-add-source = Add
package-sources-note = A package sourced from a list holds what that list holds, and is not offered where songs are added to a package one at a time. A working list is offered to no package: songs somebody set aside to decide about later are not a volume to build.
sync-button = { $count ->
    [one] Sync from { $count } list…
   *[other] Sync from { $count } lists…
  }
sync-note = Adds what the lists hold and takes out what they no longer do. Every song that stays keeps its number.
sync-no-sources-note = Add a list above to have this package hold what that list holds.
sync-from = from
sync-adding = { $count ->
    [one] { $count } goes in
   *[other] { $count } go in
  }
sync-removing = { $count ->
    [one] { $count } comes out
   *[other] { $count } come out
  }
sync-keeping = { $count ->
    [one] { $count } stays where it is
   *[other] { $count } stay where they are
  }
sync-starts-volumes = { $count ->
    [one] every volume is full, so this starts a new one
   *[other] every volume is full, so this starts { $count } new ones
  }
from-filter-keep-sourced = and keep it sourced from

package-raise-first-build = raise the version on every build after this one
package-raise-to = raise the version to { $version } on this build
package-raise-not-three-numbers = raise the version — “{ $version }” is not three numbers, so it cannot be

## Settings ---------------------------------------------------------------------------

settings-machine = Machine
settings-base-url = base URL
settings-save-and-check = Save and check
settings-discover = Discover
settings-discover-note = Lists machines found on this network. Nothing changes until you press “Select”.
settings-machine-used-for = Used to test-play a song and to install a finished package.
settings-here-lead = This machine is on this computer, so test-play passes it the file path and copies nothing. It plays only files inside the folders listed in its
settings-here-tail = setting. That list starts empty, so the first test-play on a new install is refused.
settings-elsewhere-lead = This machine is elsewhere on the network, so test-play uploads the song, which can take a while for a video. It accepts uploads only when its
settings-elsewhere-tail = setting is on. That setting starts off, so the first test-play on a new machine is refused.
settings-which-setting-lead = The error message says which setting to change. Run
settings-on-that-machine = on that machine
settings-which-setting-tail = to find the settings file.

settings-backup = Backup
settings-backup-to = backup to
settings-write-backup = Write the backup
settings-hand-set = { $count ->
    [one] { $count } song carries something you typed.
   *[other] { $count } songs carry something you typed.
  }
settings-restore-from = restore from
settings-overwrite = overwrite
settings-restore = Restore
settings-restore-confirm = Restore from this file?

settings-suggested-tags = Suggested tags
settings-comma-separated = comma-separated
settings-kept-in = Kept in
settings-kept-in-tail = , which you can also edit by hand.

settings-this-folder = This folder
settings-root = Root
settings-files = Files
settings-deleted = Thrown away
settings-did-not-parse = { $count ->
    [one] { $count } of which did not parse
   *[other] { $count } of which did not parse
  }

## The songs page's filter bar ---------------------------------------------------------
#
# `songs-language` is the language a song is *sung* in. What language these pages are in is
# `settings-locale-`, and the two never share a word.

songs-find-placeholder = title or artist
songs-find-title = part of a title or an artist; quotation marks ask for the words in that order
songs-only-this-artist = only this artist
songs-artist-title = every song by exactly this performer
songs-sort = sort
songs-sort-title = title
songs-sort-artist = artist
songs-sort-suitability = suitability
songs-sort-length = length
songs-sort-updated = recently updated
songs-sort-added = recently added
songs-your-score = your score
songs-your-score-title = the rating you set, in the Score column
songs-copies = copies
songs-copies-title = how many byte-identical files on disk this song has
songs-language = language
songs-language-not = leave out
songs-language-not-title = hide every song in this language, on top of any already left out
songs-show-filenames = show file names
songs-show-filenames-title = show each song's file name beside its title
songs-show-warnings = show file warnings
songs-show-warnings-title = show what the analysis found wrong with each song
songs-band-quality = Quality
songs-suitability-title = the quality of the source file
songs-any = any
songs-unset = unset
songs-set = set
songs-melody-channel = melody channel
songs-melody-found = found
songs-melody-not-found = not found
songs-band-song = song
songs-media-type = media type
songs-media-type-title = MIDI, video, MP3+G, or all of them
songs-any-if-set = any if set
songs-tags = tags
songs-add-tag-title = widen to songs carrying this tag as well as the ones already chosen
songs-add-one = add one
songs-suggested = suggested
songs-lyrics = lyrics
songs-per-syllable = per syllable
songs-per-line = per line
songs-lyrics-none = none
songs-encoding = encoding
songs-encoding-title = fallback is the CP1252 guess, around a third of a real corpus, and the text worth eyeballing
songs-encoding-guessed = guessed
songs-encoding-detected = detected
songs-encoding-pinned = pinned
songs-band-state = state
songs-favorite = favorite
songs-filed = filed
songs-filed-title = whether a song is in a favorite, in one that is a filing, in none, in no filing, or either
songs-in-any-favorite = in any favorite
songs-in-filed-favorite = in a filing, not a working list
songs-in-no-favorite = in no favorite
songs-in-no-filed-favorite = in no filing, working lists aside
songs-more-than-ten = more than 10
songs-added = added
songs-added-title = when a scan first found this song
songs-added-day = in the last day
songs-added-week = in the last 7 days
songs-added-month = in the last 30 days
songs-added-over-month = more than 30 days ago
songs-every-version = every version
songs-every-version-title = show every file of a recording, not just the best copy of each
songs-not-packaged = not packaged
songs-only-deleted = only deleted
songs-only-deleted-title = show what has been thrown away instead of the corpus

kind-midi = MIDI
kind-video = video
kind-cdg = MP3+G
kind-ultrastar = UltraStar

## The curation tabs --------------------------------------------------------------------

songs-tab-language = Language
songs-tab-tags = Tags
songs-tab-titles = Titles
songs-tab-analysis = Analysis
songs-tab-delete = Delete
songs-scope-ticked = the ticked songs
songs-scope-matching = every matching song
songs-to = to
songs-set-language-of = set the language of
songs-only-if-not-set = only if not set
songs-tag-add = add
songs-tag-remove = remove
songs-delete = throw away
songs-undelete = bring back
songs-delete-note = a song thrown away is in no list but "only deleted", and its files are not read again
songs-the-tag = the tag
songs-tag-title = ASCII letters, digits and - only; accents fold away, so "Forro" with an accent folds to "forro"
songs-apply = Apply
songs-make-package-called = make a package of every matching song, called
songs-make = Make
songs-add = add
songs-no-packages-lead = No packages yet - make one on the
songs-no-favorites-lead = No favorites yet - make one on the
songs-page-tail = page.
songs-put-into = put into
songs-take-out-of = take out of
songs-title-from-filename = Title from file name
songs-title-from-filename-note = Replace the title of the ticked songs with its file name, and clear the artist the file declared.
songs-fix-capitals = Fix the capitals
songs-fix-capitals-note = Give the title and artist of the ticked songs a capital on each word, leaving short words such as "the" and "de" small. A first pass: a name you typed that already mixes capitals is left alone.
songs-split-artist = Artist from the title
songs-split-artist-note = Split the title of the ticked songs at its first "-", putting what comes before it in the artist. Only a song with nothing in its artist.
songs-recalculate-of = recalculate the suitability of
songs-recalculate = Recalculate
songs-recalculate-note = Reads each file again. What you typed is left alone.
songs-hint = Hint which to play first
songs-hint-clear = Clear
songs-hint-note = Numbers the ticked MIDI songs 1, 2, 3... The list does not move and nothing is saved.

## A song's own page ---------------------------------------------------------------------
#
# `songs-tab-language` labels the language a song is *sung* in. Which language these pages are in is
# `settings-locale-`, and the two never share a word.

song-back = back to the list
song-no-title-title = no title in the file - showing its name
song-test-play = Test-play
song-open-in-os = Open in OS
song-similar = Similar names
song-words = Same words
song-download = Download
song-youtube = YouTube
song-favorites-title = the favorites this song is in

song-merged-lead = This song is marked as the same recording as
song-another-song = another song
song-merged-tail = , so it is hidden from browsing.
song-unmerge = Unmerge
song-duplicate-lead = Another file looks like the same recording and reads better, so the song list shows
song-that-one = that one
song-duplicate-tail = instead of this. Nobody decided this; it was worked out from the two files.
song-show-anyway = Show it anyway
song-show-anyway-title = browse this file on its own again, until the next suggestion pass

song-tab-details = Details
song-tab-files = Files
song-tab-filing = Filing
song-tab-lyrics = Lyrics
song-tab-advanced = Advanced

# The Advanced tab's words control. A file can be a good arrangement and a bad karaoke song, and
# only somebody listening can tell the difference in the cases the analysis cannot name.
song-lyrics-heading = Words on screen
song-lyrics-automatic = Automatic
song-lyrics-automatic-hidden = not shown, because there is nothing to follow
song-lyrics-automatic-shown = shown
song-lyrics-show = Always show them
song-lyrics-hide = Never show them
song-lyrics-note =
    The machine plays the song either way. Turn the words off for a file whose lyrics are mistimed,
    are the arranger's own details, or belong to another song; the television then says "no lyrics"
    in the corner. Rebuild the package for a change here to reach a machine.

package-tab-songs = Songs
package-tab-sources = Sources
package-tab-build = Build

song-file-says-nothing = (the file says nothing)
song-language-declared = Unknown, so this song counts as { $name } - the file's own header says { $code }.
song-language-declared-default = Unknown, so this song counts as { $name } - the file's own header says { $code }, which is what most karaoke files say whatever language they are in.
song-language-from-encoding = Unknown, so this song counts as { $name } - worked out from the encoding its lyrics are written in.
song-language-guessed = Unknown, so this song counts as { $name } - read from its own words, { $percent }% sure.
song-language-unknown-code = The file's header says { $code }, which is not a language code this build knows.

song-transpose = Transpose
song-semitones = semitones, applied by default
song-notes = Notes
song-correction-note = applied every time this song plays, here and on the machine

song-file-analysis = File analysis
song-file-info = File info
song-suitability-parts = lyrics { $lyrics }/3 - sync { $sync }/3 - channels { $channels }/2 - arrangement { $arrangement }/2
song-native-karaoke = Native karaoke file.
song-your-rating = Your rating
song-melody-channel = channel { $channel }
song-melody-channel-confidence = channel { $channel } ({ $confidence } confidence)
song-melody-not-found = not found
song-melody-no-candidates = not found - no instrument plays any notes
song-melody-nothing-monophonic = not found - no channel plays one note at a time
song-melody-outside-vocal-range = not found - every channel that plays one note at a time is outside singing range
song-melody-silent-under-the-words = not found - no channel plays while the words are sung
song-melody-no-supporting-evidence = not found - nothing ties a channel that plays one note at a time to the words
song-melody-ambiguous = not found - two or more channels fit equally well
song-format = Format
column-length-full = Length
song-content-label = Content
song-content = { $notes } notes across { $channels } channels - { $lines } lines, { $syllables } syllables
song-encoding = Encoding
song-encoding-guess = A guess. Read the lyrics on the Lyrics tab and pin the right one if the text looks wrong.
song-picture = Picture
song-codecs = Codecs
song-audio = audio
song-graphics-file = Graphics file
song-graphics = Graphics
song-cdg-length = words run { $words }
song-cdg-audio = { $channels ->
    [one] { $channels } channel
   *[other] { $channels } channels
  }
song-cdg-graphics = { $tiles } tiles - { $packets } packets
song-cdg-unknown = { $count ->
    [one] { $count } packet uses a CD+G instruction this build does not implement.
   *[other] { $count } packets use a CD+G instruction this build does not implement.
  }
song-added = Added
song-hash = Hash
song-warnings = Warnings

song-files-heading = { $count ->
    [one] Files on disk
   *[other] Files on disk - { $count } identical copies
  }
song-no-files = No file for this song is under the root any more. It is kept because a package still names it; re-scan after restoring the folder, or remove it from the package.
song-bytes = bytes
song-folder-title = every song under this folder, subfolders included
song-songs-in-folder = songs in this folder
song-versions-heading = { $count ->
    [zero] Other versions
   *[other] Other versions - { $count }
  }
song-no-other-versions = No other file here looks like this recording.
song-other-versions-note = Different bytes, the same shape and the same name. The browse list shows one of these and hides the rest, so nothing gets curated twice. Which of them is really the same recording is answered by listening, and that is what the play buttons are for.
song-test-play-version-title = test-play it, to hear whether it is the same recording
song-not-the-same = Not the same
song-not-the-same-title = stop grouping these two, for good

song-tag-title = whatever you type is folded to a slug, and only ASCII letters, digits and - survive
song-no-favorites-defined = None defined.
song-save-favorites = Save favorites
song-no-packages = Not in any package.
song-as-number = as number
song-add-to-package = Add to package
song-replace-number = Put in place of number
song-replace-in = in
song-replace = Replace
song-replace-title = Give this song the number another song has in a package. The other song leaves the package, and the number stays the same.

song-decode-as = decode as
song-show = Show
song-loading = loading
song-text-events = Text events in this file
song-show-raw = Show the raw text

song-channels = Channels
song-channel = Channel
song-instrument-in-file = Instrument in the file
song-notes-column = Notes
song-track = Track
song-ignore-bank = Ignore bank select
song-silence = Silence
song-recentre-bend = Fix a stuck bend
song-recentre-bend-title = This part bends a note and does not bend it all the way back, so later notes play out of tune. Tick to put the bend back before those notes.
song-play-as = Play as
song-drums = drums
song-no-melody = no melody
song-melody-is-this = the melody is on this channel
song-a-kit = a kit, not an instrument
song-save-corrections = Save corrections

## What an action answers with -----------------------------------------------------------
#
# Each of these is said from a handler, in the language the page that pressed the button is being
# drawn in. A sentence carrying a count or a name is composed with arguments instead; those sit with
# the page they belong to.

said-added-to-favorite = Added to that favorite.
said-build-it-first = Build it first — there is no file to install.
said-choose-a-favorite = Choose a favorite first.
said-corrections-saved = Corrections saved.
said-favorite-created = Created { $name }. Star songs into it from the Songs page.
said-favorite-renamed = Renamed to { $name }.
said-favorite-deleted = Deleted. The songs it held are untouched.
said-favorite-gone = That favorite is gone. Reload the page.
said-filter-already-gone = That one is already gone.
said-filter-renamed = Renamed to { $name }.
said-filter-saved = Saved as { $name }.
said-filter-saved-not-drawn = Saved { $name }. The strip could not be redrawn: { $error }
said-filter-updated = { $name } was updated.
said-forgotten = Forgotten.
said-in-no-favorites = In no favorites.
said-name-first = Name it first.
said-name-the-backup = Name the backup file to restore from.
said-name-the-package = Give the package a name first.
said-no-folder-given = No folder given.
said-no-folder-open = No folder is open.
said-no-numbers-to-clear = There are no numbers to clear.
said-not-a-number = That is not a number.
said-now-number = Now number { $number }.
said-choose-a-package = Choose a package first.
confirm-replace = Number { $number } in { $package } is { $old }. Put { $new } in its place?
confirm-replace-favorites = { $count ->
    [one] This package follows the list { $favorites }, so { $new } also takes its place there.
   *[other] This package follows the lists { $favorites }, so { $new } also takes its place in them.
  }
said-replaced = { $new } is now number { $number } in { $package }, in place of { $old }.
said-replaced-in-favorites = It also took its place in { $favorites }.
said-replace-empty = No song has number { $number } in { $package }.
said-replace-same-song = This song already has number { $number } there.
said-replace-already-in = This song is already in that package, as number { $number } in { $package }. Remove it there first, or replace another song with it.
said-replace-merged = This song was merged into another song. Replace with that song instead.
said-replace-deleted = This song was thrown away. Bring it back first, or choose another song.
said-nothing-change = Nothing to change.
said-nothing-is-ticked = Nothing is ticked.
said-nothing-matches-filter = Nothing matches that filter.
said-nothing-to-drop = Nothing to drop: every song in it is a different recording.
said-nothing-was-ticked = Nothing was ticked.
said-opened-in-browser = Opened in your browser.
said-package-created = Created { $name }. Open it to choose what goes in and to build it.
said-package-deleted = Deleted. Any .kmpkg already written is untouched.
said-rated = Rated { $value }/10.
said-rating-cleared = Rating cleared.
said-lyrics-hidden = The words will not be shown while this song plays.
said-lyrics-shown = The words will be shown while this song plays.
said-lyrics-automatic = The words follow what the analysis finds.
said-removed-from-favorite = Removed from that favorites folder.
said-removed = Removed from this package. The song itself is untouched.
said-renumbered = { $count ->
    [one] Renumbered { $count } song from this package’s first number, keeping the order.
   *[other] Renumbered { $count } songs from this package’s first number, keeping the order.
  }
said-saved = Saved.
said-volume-format-refused = A volume name has to hold {"{"}n{"}"}, where the number goes, or every volume would be written to one file. “{ $format }” was not saved.
said-saved-no-tags = Saved. No tags are suggested now.
said-suggestion-pass-unfinished = the suggestion pass did not finish
said-type-a-tag = Type a tag first.
said-unmerged = Unmerged. It will appear in browsing again.

## ...and what an action answers with when it counts something ---------------------------

said-language-set = { $count ->
    [one] Set the language of { $count } song.
   *[other] Set the language of { $count } songs.
  }
said-tag-set = { $count ->
    [one] Set the tag on { $count } song.
   *[other] Set the tag on { $count } songs.
  }
said-tag-removed = { $count ->
    [one] Removed the tag on { $count } song.
   *[other] Removed the tag on { $count } songs.
  }
said-deleted = { $count ->
    [one] Threw away { $count } song.
   *[other] Threw away { $count } songs.
  }
said-undeleted = { $count ->
    [one] Brought back { $count } song.
   *[other] Brought back { $count } songs.
  }
said-filed = { $count ->
    [one] Filed { $count } song.
   *[other] Filed { $count } songs.
  }
said-took-out = { $count ->
    [one] Took out { $count } song.
   *[other] Took out { $count } songs.
  }
said-re-reading = { $count ->
    [one] Re-reading { $count } file. Watch it on the Scan page.
   *[other] Re-reading { $count } files. Watch it on the Scan page.
  }
said-cleared-numbers = { $count ->
    [one] Cleared { $count } number.
   *[other] Cleared { $count } numbers.
  }
said-not-a-tag = { $typed } is not a tag — a tag is ASCII letters, digits and a hyphen. Accents fold away, so an accented o folds to a plain one; anything else does not.
said-encoding-pinned = Pinned { $encoding }. Every package built from this song will decode it that way.
said-now-a-working-list = { $name } is a working list. Its songs no longer count as filed.
said-now-a-filing = { $name } is a filing again.
said-dropped-second-copies = { $count ->
    [one] Dropped { $count } entry. The best copy this list held of each song stayed.
   *[other] Dropped { $count } entries. The best copy this list held of each song stayed.
  }
said-grouping-done = { $groups } groups look like one recording, hiding { $hidden } songs. { $pairs } pairs.
said-version-refused = A version is three numbers with dots between them — 1.0.0, say. “{ $version }” is not one, and was not saved.
said-start-number-refused = A package numbers its songs 1 to { $highest }; the machine adds the bank, so a first number above that would be dialled as another package's. { $number } was not saved.
said-package-full = { $name } has no numbers left. A package holds { $highest } songs and this one is full.
said-name-has-no-file-name = { $name } leaves nothing that can be a file name. Try a name with letters or digits in it.
said-spec-written = { $count ->
    [one] Wrote { $file } describing { $count } song. Edit it in any text editor and build it with km-pack, or go on building from this page, which reads the same description without the file.
   *[other] Wrote { $file } describing { $count } songs. Edit it in any text editor and build it with km-pack, or go on building from this page, which reads the same description without the file.
  }
said-build-gone = { $file } is not there any more. Build it again.
said-imported-all = { $count ->
    [one] Imported { $package } with { $count } song, matched to a file in this folder.
   *[other] Imported { $package } with { $count } songs, all matched to files in this folder.
  }
said-imported = { $count ->
    [one] Imported { $package } with { $count } song.
   *[other] Imported { $package } with { $count } songs.
  }
said-machine-reached = Saved. Reached “{ $name }” at { $url }.
said-machine-saved-no-answer = Saved { $url }, but no machine answered there yet: { $why }
said-backup-written = Wrote { $file }: { $songs ->
    [one] { $songs } song
   *[other] { $songs } songs
  } carrying something you typed, and { $favorites ->
    [one] { $favorites } favorite
   *[other] { $favorites } favorites
  }. Keep it somewhere other than this folder — it is the half of this corpus a re-scan cannot rebuild.
said-restored = Restored { $songs ->
    [one] { $songs } song
   *[other] { $songs } songs
  }, filed { $filed } into { $favorites ->
    [one] { $favorites } new favorite
   *[other] { $favorites } new favorites
  }, and recorded { $merges ->
    [one] { $merges } merge
   *[other] { $merges } merges
  }.
said-restored-later-format = This file was written by a later build (format { $format }); everything this one understands was read anyway.

## What a chip says a filter is doing -----------------------------------------------------
#
# A language's own name is not here: `km_kmpkg::Language` carries an English name for the pickers
# that have to show it, and that is data about the corpus.

chip-by = by { $artist }
chip-starts-with = starts with { $letter }
chip-starts-with-a-number = starts with a number
chip-starts-with-a-symbol = starts with a symbol
initial-any = any
initial-symbol = symbol
chip-suitability-high = suitability 8–10
chip-suitability-middle = suitability 5–7
chip-suitability-low = suitability under 5
chip-suitability-range = suitability { $range }
chip-score-set = { $name } set
chip-score-unset = { $name } unset
chip-score-at-least = { $name } ≥ { $score }
chip-melody-found = melody found
chip-melody-abstained = melody abstained
chip-midi-only = MIDI only
chip-video-only = video only
chip-cdg-only = MP3+G only
chip-language-unset = language unset
chip-language-any = any language
chip-language-not = not { $language }
chip-tag = tag: { $tag }
chip-lyrics-per-syllable = lyrics per syllable
chip-lyrics-per-line = lyrics per line
chip-no-lyrics = no lyrics
chip-in-favorite = in { $name }
chip-one-copy = one copy
chip-two-to-ten-copies = 2–10 copies
chip-over-ten-copies = more than 10 copies
chip-added-day = added in the last day
chip-added-week = added in the last 7 days
chip-added-month = added in the last 30 days
chip-added-over-month = added more than 30 days ago
chip-more-than-one-copy = more than one copy

## ...and the rest of what an action answers with ------------------------------------------

said-no-address-yet = The tool does not know its own address yet.
said-nothing-ticked-on-disk = Nothing is ticked, or no ticked song still has a file on disk.
said-nothing-matching-on-disk = Nothing matches that filter, or none of it still has a file on disk.
said-titles-taken = { $count ->
    [one] Took the title of { $count } song from its file name and cleared the artist.
   *[other] Took the title of { $count } songs from their file names and cleared the artist.
  }
said-titles-taken-short = { $count ->
    [one] Took the title of { $count } song from its file name and cleared the artist; { $short } had no file name to take.
   *[other] Took the title of { $count } songs from their file names and cleared the artist; { $short } had no file name to take.
  }
said-capitals-fixed = { $count ->
    [one] Fixed the capitals of { $count } song.
   *[other] Fixed the capitals of { $count } songs.
  }
said-capitals-fixed-short = { $count ->
    [one] Fixed the capitals of { $count } song; { $short } already had capitals of their own.
   *[other] Fixed the capitals of { $count } songs; { $short } already had capitals of their own.
  }
said-artist-split = { $count ->
    [one] Took the artist out of the title of { $count } song.
   *[other] Took the artist out of the title of { $count } songs.
  }
said-artist-split-short = { $count ->
    [one] Took the artist out of the title of { $count } song; { $short } named an artist already or had no "-" in the title.
   *[other] Took the artist out of the title of { $count } songs; { $short } named an artist already or had no "-" in the title.
  }
said-nothing-ticked-is-midi = Nothing ticked is a MIDI file, and a video or an MP3+G song is a 10 by what it is rather than by measurement.
said-numbered = { $count ->
    [one] Numbered { $count } song, best first.
   *[other] Numbered { $count } songs, best first.
  }
said-numbered-skipped = { $count ->
    [one] Numbered { $count } song, best first. { $skipped } are not MIDI files and have nothing to compare.
   *[other] Numbered { $count } songs, best first. { $skipped } are not MIDI files and have nothing to compare.
  }
said-handed-to-opener = Handed { $file } to the system opener. That happens on the machine running the package builder, so if your browser is somewhere else, use Download instead.
said-could-not-open = could not open it: { $why }
said-worker-died = the worker thread died: { $why }
said-shown-again = Shown in the song list again. The next pass sets it aside; dismiss the pair instead.
said-pair-dismissed = Noted. They will not be grouped again, and both browse on their own.

## What a build, an import or a restore reports -------------------------------------------

said-package-needs-a-name = A package needs a name — it is what you will see it by, and the id is generated.
said-default-language-set = Saved. Songs with no language of their own go in as { $code } — in the package only; nothing is written back to the songs.
said-default-language-cleared = Saved. A song with no language will now stop the build and be listed for you to classify.
said-package-made = { $count ->
    [one] Made { $package } with { $count } song. It is on the Packages page, where it is built.
   *[other] Made { $package } with { $count } songs. It is on the Packages page, where it is built.
  }
said-package-made-sourced = It stays sourced from that list, so it is not offered where songs are added to a package one at a time, and Sync on its own page is what keeps the two together.
said-package-took-the-first = The filter matched more songs than a package holds, so the first { $count } in this order went in.
said-package-no-room = { $count ->
    [one] { $count } of them had no number left in it.
   *[other] { $count } of them had no number left in it.
  }
said-name-a-list = Choose a list first.
said-source-added = Now sourced from { $name }. Press Sync to have the package hold what its lists hold.
said-source-removed = No longer sourced from { $name }. The songs it put in are still there until the next sync.
said-source-is-a-working-list = { $name } is a working list, and a package is offered none: songs somebody set aside to decide about later are not a volume to build. Take the working-list mark off it on the Favorites page, or choose another.
said-sources-cleared = This package is sourced from no list now, so it is an ordinary package again and can be added to a song at a time. The songs it holds stay where they are.
said-no-sources-to-sync = This package is sourced from no list, and a sync of one would empty it. Add a list first.
said-nothing-to-sync = { $count ->
    [one] Already holds what its lists hold — 1 song, nothing to add and nothing to take out.
   *[other] Already holds what its lists hold — { $count } songs, nothing to add and nothing to take out.
  }
said-synced = Synced: { $added } in, { $removed } out, { $kept } kept their numbers.
said-sync-new-volumes = { $count ->
    [one] The lists outgrew the package, so it has a new volume.
   *[other] The lists outgrew the package, so it has { $count } new volumes.
  }
said-sync-clashed = { $count ->
    [one] { $count } of them is another file of a song this package already had — kept, because two takes of one song can be two songs. Tidy on the Favorites page is what drops the second copies.
   *[other] { $count } of them are another file of a song this package already had — kept, because two takes of one song can be two songs. Tidy on the Favorites page is what drops the second copies.
  }
said-package-is-sourced = { $count ->
    [one] That package holds what { $favorites } holds, so a song put in here would go out again on its next sync. File the song in that list instead.
   *[other] That package holds what { $favorites } hold, so a song put in here would go out again on its next sync. File the song in one of those lists instead.
  }

said-build-stopped = Stopped before anything was written. The package it was replacing is untouched.
said-build-volume = Volume { $number }:
said-build-nothing-readable = Nothing was written: no song in this package could be read.
said-build-manifest-problems = Not written — the manifest has problems:
said-build-unlanguaged = { $count ->
    [one] Not written — { $count } song has no language, and a package cannot ship without one:
   *[other] Not written — { $count } songs have no language, and a package cannot ship without one:
  }
said-build-unlanguaged-ways-out = Either set “unclassified songs are” above — which fills them in this package only and writes nothing back to the songs — or classify them for good on the Songs page: filter to the songs with no language, add the folder they are in, and use “set the language of every matching song”.
said-build-written = { $count ->
    [one] Wrote { $file }, version { $version }, with { $count } song
   *[other] Wrote { $file }, version { $version }, with { $count } songs
  }
said-build-listing-written = The song list is in { $file }.
said-build-re-encoded = { $count ->
    [one] { $count } video re-encoded
   *[other] { $count } videos re-encoded
  }
said-build-copied = { $count ->
    [one] { $count } copied as it was
   *[other] { $count } copied as they were
  }
said-build-cdg-pairs = { $count ->
    [one] { $count } MP3+G pair
   *[other] { $count } MP3+G pairs
  }
said-build-ultrastar = { $count ->
    [one] { $count } UltraStar song
   *[other] { $count } UltraStar songs
  }
said-build-left-out = { $count ->
    [one] { $count } song left out:
   *[other] { $count } songs left out:
  }
said-and-more = ... and { $count } more
said-import-unmatched = { $count ->
    [one] { $count } could not be matched to anything under this root:
   *[other] { $count } could not be matched to anything under this root:
  }
said-import-unreadable-language = { $count ->
    [one] { $count } song names a language this build cannot read, so it was not imported:
   *[other] { $count } songs name a language this build cannot read, so it was not imported:
  }
said-install-already-in-catalog = { $count ->
    [one] { $count } song already in the catalog:
   *[other] { $count } songs already in the catalog:
  }
said-restore-unmatched = { $count ->
    [one] { $count } song in the file is not in this folder — scan first, then restore again:
   *[other] { $count } songs in the file are not in this folder — scan first, then restore again:
  }
said-restore-rejected = { $count ->
    [one] { $count } value this build would not take:
   *[other] { $count } values this build would not take:
  }
said-type-the-password = Type the machine's password first.
said-signed-in-with-saved = Signed in with the password this computer had saved.
said-signed-in-remembered = Signed in. This computer will remember the password for this machine.
said-signed-in-not-written = Signed in. The password is not written down.
said-signed-out = Signed out, and nothing is remembered for this machine.

said-package-name-taken = There is already a package called { $name }. Give this one another name.
said-debugging-on = Debugging will be on when this machine next restarts. Until then its two play routes are not mounted, so a test-play is still refused.
said-debugging-off = Debugging will be off when this machine next restarts.

## When something goes wrong ---------------------------------------------------------------
#
# The `Display` on each error stays English: that is what the log carries. What a page says is here.

db-error-sqlite = The database answered with an error: { $why }
db-error-not-found = There is no { $what } here.
db-error-no-folder = No folder is open.
db-error-busy = The corpus is being written to. Try that again in a moment.

# The page a refused navigation becomes. The sentence above is what it prints; these are the words
# around it, and the two controls are the way out that a plain sentence did not have.
error-page-title = This page could not be drawn
error-try-again = Try again
error-back-to-songs = Go to the songs
error-will-retry = This page asks again by itself every few seconds.
app-error-unreachable = The karaoke machine is not answering at { $url } ({ $why }).
app-error-unexpected = The karaoke machine answered { $status }: { $body }
said-folder-not-listed = The folder could not be listed: { $why }
row-path-copies = { $path }
    { $count } copies

## The window a failure before the page shows ----------------------------------------------

window-could-not-start = { $program } could not start

said-machine-reached-at = Reached “{ $name }” at { $url }.
said-machine-holds = { $count ->
    [one] It has { $count } song installed.
   *[other] It has { $count } songs installed.
  }
said-machine-stale = No machine has answered at this address for six hours or more. If this corpus has moved to another computer, press Discover and choose a machine on this network.
