# km-song-sync: every word its page says, in English.
#
# A message with a `{ $variable }` is filled in by the Rust. A template names only messages with
# none.

app-title = KaraokeMachine Song Sync
language-picker = Language
action-quit = Quit

home-heading = Put words on a MIDI file
home-intro = Paste the words of a song, find its MIDI file below, and press Start. The sync editor opens, and you tap each syllable in time with the music. The editor saves a new .kar file beside the song and never changes the song.
home-no-machine = The karaoke machine was not found beside this program, so nothing can be started. Install the machine, or name it with --machine-exe.

words-label = The words
words-placeholder = One line of the song on each line. Leave an empty line between verses.
words-hint = A hyphen splits a word into syllables: ka-ra-o-ke is four taps. Type \- for a hyphen that is sung. Leave the box empty to use the text file beside a song, or the words the song already has.
words-continue = Keep the timing the song already has, and tap only the words after it
action-clear = Clear

action-start = Start
action-reveal = Show in folder
action-go = Go

browse-top-title = Go to the top
browse-up-title = Go up one folder
browse-this-computer = This computer
browse-folder-label = The full path of a folder
browse-named = Named
browse-part-of-a-name = Part of a name
browse-nothing-found = There are no folders and no MIDI files here.
browse-nothing-called-that = Nothing here has that in its name.
browse-range = { $first } to { $last } of { $count }
page-previous = Previous
page-next = Next

row-not-midi = This is not a MIDI file the editor can read.
row-uses-text-file = Uses the words in { $file } when the box above is empty.
row-own-words = Has words already. With the box above empty, the editor opens them for correction.
row-needs-words = Has no words. Paste them in the box above.
row-output-exists = { $file } is already here.
row-replace = Replace it

editor-running = The editor is open on { $song }.
editor-running-hint = Press Ctrl+S in the editor to save, and close its window when you finish.
editor-saved = Saved { $file }.
editor-saved-nothing = The editor closed on { $song } and saved nothing.
editor-failed = The editor did not open { $song }.

said-busy = The editor is already open. Close it before you start another song.
said-no-song = That is not a MIDI file in a folder this program can read.
said-no-machine = The karaoke machine was not found, so the editor cannot start.
said-no-words = That song has no words. Paste them in the box first.
said-not-midi = That is not a MIDI file the editor can read.
said-output-exists = The synced file is already there. Tick Replace it on that row first.
said-reveal-failed = The folder could not be opened.
