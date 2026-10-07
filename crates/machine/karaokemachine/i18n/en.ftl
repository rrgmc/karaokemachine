# What the lyric sync editor says on its own screen. See `src/sync.rs`.
#
# The key caps (Space, Ctrl+S) are in the source, since they are what is printed on the keys.

## The window and the line at its head

sync-window-title = Lyric sync - { $title }
sync-mode-tapping = TAPPING
sync-mode-review = REVIEW
sync-playing = Playing
sync-paused = Paused
sync-status = { $transport }   { $position } / { $length }   tempo { $tempo }%   { $tapped } of { $total } tapped
sync-status-unsaved = { $status }   not saved
sync-channel = channel { $number }
sync-channel-named = channel { $number } ({ $name })
sync-vocal-label = Vocal line: { $channel }
sync-vocal-label-silenced = Vocal line: { $channel }, silenced
sync-vocal-label-none = Vocal line: not chosen
# The selected word inside its line, and where it starts.
sync-selected-at = { $line }   at { $time }

## Messages

sync-start = Press Enter to start the song, then Space on each word
# The editor was opened on a file's own words, and on a file to go on tapping.
sync-reopened = These are the words the file has, with their timing. Move a word, then Ctrl+S saves
sync-continued = { $tapped } words keep their timing. Enter plays from the next word
sync-unsaved-close = Not saved. Ctrl+S saves, closing again leaves
sync-unsaved-esc = Not saved. Ctrl+S saves, Esc again leaves
sync-saved = Saved { $tapped } of { $total } words to { $file }
sync-not-saved = Not saved: { $reason }
sync-nothing-to-review = Nothing is tapped yet, so there is nothing to review
sync-all-tapped-already = Every word is tapped, so no word is left to tap
sync-tapping-again = Tapping again, from the next word
sync-review-so-far = Review of the { $tapped } words tapped so far. R goes back to tapping
sync-all-tapped = All words tapped. The song now repeats with your timing, to check
sync-paused-tap = The song is paused. Enter plays it
sync-word-ends = "{ $word }" ends here
sync-end-refused = E ends a word after it starts, while the song plays
sync-vocal-is = The vocal line is { $channel }. V silences it, to hear what is left
sync-vocal-is-silenced = The vocal line is { $channel }, and it is silenced
sync-no-vocal = No vocal line is chosen. M picks the channel
sync-silenced = { $channel } is silenced. V brings it back
sync-sounds-again = { $channel } sounds again
sync-taps-follow = Your taps follow { $channel }, so it is the vocal line. M changes it
sync-snapped = { $moved } words moved onto the notes of { $channel }. Ctrl+Z takes it back
sync-snap-undone = The words are back where you tapped them
sync-no-audio = No audio output. Trying again
sync-audio-back = The audio output is back

## The key list: what a row's keys act on, then what each key does

sync-row-tap = TAP
sync-row-word = WORD
sync-row-song = SONG
sync-row-notes = NOTES
sync-row-file = FILE
sync-key-next-word = next word
sync-key-end-word = end the word
sync-key-undo = undo
sync-key-tap-line-again = tap this line again
sync-key-play-pause = play / pause
sync-key-seek = 5 s
sync-key-tempo = slower / faster
sync-key-vocal-next = next channel as the vocal line (Shift: previous)
sync-key-silence = silence it
sync-key-save = save
sync-key-leave = leave
sync-key-hide-keys = hide these keys
sync-key-show-keys = keys
sync-key-review-so-far = review what is tapped so far
sync-key-back-to-tapping = back to tapping
sync-key-select = select
sync-key-select-sung = select the word being sung
sync-key-end-here = end here
sync-key-move = move 10 ms (Shift 50)
sync-key-play-line = play this line
sync-key-snap = move the words onto its notes (Ctrl+Z undoes)
