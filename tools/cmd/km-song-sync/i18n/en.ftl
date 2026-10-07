# km-song-sync: every word its page says, in English.
#
# A message with a `{ $variable }` is filled in by the Rust. A template names only messages with
# none.

app-title = KaraokeMachine Song Sync
language-picker = Language
action-quit = Quit

home-heading = Put words on a MIDI file
home-intro = Find the MIDI file of a song and press Select. Then paste its words and press Start. The sync editor opens, and you tap each syllable in time with the music. The editor saves a new .kar file beside the song and never changes the song.
home-no-machine = The karaoke machine was not found beside this program, so nothing can be started. Install the machine, or name it with --machine-exe.

words-label = The words
words-placeholder = One line of the song on each line. Leave an empty line between verses.
words-hint = A hyphen splits a word into syllables: ka-ra-o-ke is four taps. Type \- for a hyphen that is sung. Without the tick, a song uses the text file beside it, or the words it already has.
words-continue = The song is partly tapped already: keep those words as they are, and tap only the rest
words-use = Use the words in this box
words-selected = Song:
words-selected-none = None selected. Press Select on a song in the first tab.
names-title = Title
names-artist = Artist
names-language = Language
names-language-none = No change
names-hint = These go into the synced file. Change one the song states wrongly. An empty field keeps what the song states, and a song with no title gets its file name.
tab-song = 1. Choose a song
tab-words = 2. Words and start
column-file = File
column-title = Title
column-artist = Artist
column-words = Words
action-clear = Clear

action-start = Start
action-select = Select
action-reveal = Show in folder
action-go = Go

browse-top-title = Go to the top
browse-up-title = Go up one folder
browse-this-computer = This computer
browse-folder-label = The full path of a folder
browse-place-home = Home
browse-place-icloud = iCloud Drive
browse-named = Named
browse-part-of-a-name = Part of a name
browse-nothing-found = There are no folders and no MIDI files here.
browse-nothing-called-that = Nothing here has that in its name.
browse-range = { $first } to { $last } of { $count }
page-previous = Previous
page-next = Next

row-not-midi = This is not a MIDI file the editor can read.
row-uses-text-file = Uses the words in { $file }.
row-own-words = Has words already.
row-output-exists = { $file } is already here.
words-replace = Replace the synced file that is already there

editor-running = The editor is open on { $song }.
editor-running-hint = Press Ctrl+S in the editor to save, and close its window when you finish.
editor-saved = Saved { $file }.
editor-saved-nothing = The editor closed on { $song } and saved nothing.
editor-failed = The editor did not open { $song }.

said-busy = The editor is already open. Close it before you start another song.
said-no-song = Select a MIDI file first.
said-no-machine = The karaoke machine was not found, so the editor cannot start.
said-no-words = That song has no words. Paste them in the box and tick it.
said-box-empty = The words box is empty. Paste the words, or take the tick off.
said-not-midi = That is not a MIDI file the editor can read.
said-output-exists = The synced file is already there. Tick the box that replaces it first.
said-reveal-failed = The folder could not be opened.
