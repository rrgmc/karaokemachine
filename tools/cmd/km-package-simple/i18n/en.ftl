# km-package-simple: every word its pages say, in English.
#
# A message with a `{ $variable }` is filled in by the Rust. A template names only messages with
# none.

app-title = KaraokeMachine Simple Package Builder
language-picker = Language
action-quit = Quit

home-heading = Make a package from a folder
home-intro = Choose a folder of karaoke files. Every MIDI file, video, MP3+G pair and UltraStar song in it and in its subfolders becomes a song in the package. You can rename songs and leave some out before you build.
home-uncurated = A package made here is marked uncurated, because nobody reviewed its songs. The machine's lists of packages show the mark. The television does not.
home-folder = Folder
home-folder-placeholder = The full path of a folder
action-read = Read the folder
action-other-folder = Choose another folder

said-failed = That did not work:
said-busy = Wait for the work in progress to finish.
said-no-folder = There is no folder at that path.
said-nothing-kept = Keep at least one song.
said-no-name = Give the package a name.
said-reveal-failed = The folder could not be opened.

progress-reading = Reading the folder: { $done } of { $total } MIDI files.
progress-building = Writing package { $volume } of { $volumes }: song { $done } of { $total }.

form-uncurated = The package is marked uncurated.
form-name = Name
form-version = Version
form-publisher = Publisher
form-language = Language for songs with none
form-out = Write to folder
form-language-hint = A song whose file names no language is filed under this one. Undetermined is the honest answer when you do not know.
action-build = Build

songs-summary = { $count ->
    [one] { $kept } of { $count } song goes in
   *[other] { $kept } of { $count } songs go in
  }, as { $volumes ->
    [one] { $volumes } package.
   *[other] { $volumes } packages.
  }
songs-range = Songs { $first } to { $last } of { $count }
songs-shift-hint = Shift-click a box to keep or leave out every song between it and the last box you clicked.
column-number = Number
column-kind = Kind
column-title = Title
column-artist = Artist
column-language = Language
column-suitability = Suitability
column-keep = Keep
page-previous = Previous
page-next = Next

kind-midi = MIDI
kind-video = Video
kind-cdg = MP3+G
kind-ultrastar = UltraStar
kind-lrc = LRC

left-heading = { $count ->
    [one] { $count } file is not a song
   *[other] { $count } files are not songs
  }
left-unreadable = could not be read.
left-not-midi = is not a MIDI file this program can read.
left-copy = holds the same song as { $detail }.
left-ultrastar = is an UltraStar file this program cannot use: { $detail }
left-lrc = is an LRC file this program cannot use: { $detail }
left-no-graphics = has no .cdg beside it, so it has no words.
left-no-audio = has no audio beside it, so there is nothing to sing over.
left-other = was left out: { $detail }

built-heading = { $count ->
    [one] Wrote { $count } package.
   *[other] Wrote { $count } packages.
  }
built-uncurated = Each one is marked uncurated.
built-listing = A list of the songs is beside each package, as a .txt file with the same name.
built-skipped = { $count ->
    [one] { $count } song did not go in
   *[other] { $count } songs did not go in
  }
count-songs = { $count ->
    [one] { $count } song
   *[other] { $count } songs
  }
action-reveal = Show in folder
action-back = Back to the songs
