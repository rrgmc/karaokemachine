# What the television says.
#
# The source catalog: every message is written here first, in US English, and every other locale is
# a translation of this file. See `The prose and the names are US English`.
#
# **Every character here has to have a glyph in the bundled font**, which is Latin coverage only —
# `every_message_is_drawable` is the test, and `crate::text::DRAWABLE_EXTRAS` is the short list of
# non-Latin-1 marks this screen already uses. HarfBuzz is off by build decision, so nothing here may
# need shaping either.
#
# **These words are read from a sofa.** Short beats complete: a sentence that has to be shrunk two
# rungs to fit is one nobody reads in the four seconds it is up.

## The keypad
#
# Digits are not here on purpose — `Label::Digit` draws them directly, because `7` is the same mark
# in every language this renders and a catalog entry for one would only be a way to break the pad.

keypad-clear = CLR
keypad-submit = OK

## The transport strip
#
# One table feeds the strip, the function-key bindings and the printed hints — see
# `input::TRANSPORT_COMMANDS`. These are the words on the buttons.

transport-pause = PAUSE
transport-back = -10s
transport-forward = +10s
transport-next = NEXT
transport-again = REPEAT
transport-queue = QUEUE
transport-key-down = KEY -
transport-key-up = KEY +
transport-melody = MELODY

## The idle screen

# The prompt under the title.
idle-prompt = Enter a song number
# A machine with nothing in it says so in words rather than showing `0 songs · 0 packages`. Zero is
# the state that needs explaining, and a row of zeroes reads as a fault in the counter.
catalog-empty = No songs installed
# The catalog summary: how much is installed. Two plurals in one line, which is why it is one
# message rather than four — a language whose rule differs from English's gets to reorder it.
catalog-summary = { $songs ->
    [one] { $songs_display } song
   *[other] { $songs_display } songs
 } · { $packages ->
    [one] { $packages_display } package
   *[other] { $packages_display } packages
 }

## A standing fault
#
# **How many and where, never why.** The whole of a reason is in `GET /packages`, on the online
# remote and in `/admin/`'s Problems tab — and the last of those can act on it, which no television
# can. See `A clash warns rather than only logging`.
#
# The areas are separate messages joined with `, ` by the caller rather than one message per
# combination: two areas are three entries to write and a third would make seven.

notice-faults = { $count ->
    [one] { $count } problem
   *[other] { $count } problems
 }: { $areas }
fault-packages = packages
fault-sound = sound

## The developer marker
#
# Short, because it shares a row with what is drawn from the left and is capped at a little under
# half the width. It is on screen for as long as the state is true, on both screens, and it is the
# only thing here whose whole purpose is that a setting cannot be left on unnoticed.
#
# **Neither of the two says anything about a password.** Both name the state and nothing else —
# naming what the switch exposes is the admin pages' job, and they have the width for it: debugging
# mounts the two `debug/play-*` routes, which are public whenever they exist, and the console serves
# the whole API again with nothing gated.

developer-debugging = DEBUG MODE ENABLED
developer-console = DEV CONSOLE ENABLED

## The queue overlay

queue-heading = Queue
queue-waiting = { $count ->
    [one] { $count } song waiting
   *[other] { $count } songs waiting
 }
queue-empty = Nothing queued — enter a song number
# One row is reserved for this whenever the queue does not fit, so the count is never a surprise.
queue-overflow = and { $count } more

## While a song is playing

# What is queued after this one.
next-up = next: { $title }
# A MIDI file that carries no words. Not drawn over a video or an MP3+G song, whose words are
# somebody else's pixels — there it would be a sentence about nothing.
no-lyrics = (no lyrics in this file)

## The badges over a playing song
#
# None of these is drawn for a video or MP3+G song, which have no key and no tempo to change.

badge-key = key { $semitones }
badge-tempo = tempo { $ratio }x
badge-melody = melody
# Said about a song whose words somebody turned off, where `no-lyrics` above is said about a file
# that has none. Two messages because a language may not phrase the two the same way.
badge-lyrics-hidden = no lyrics

## The frame meter
#
# A developer's overlay rather than a singer's screen, and translated anyway: it is drawn on the
# same television, in the same font, and a half-English panel is a worse thing to photograph than an
# English one. The measurements' units are not words.

frames-heading = FRAMES
frames-measuring = measuring...
frames-draw = draw
frames-present = present
frames-interval = interval
frames-starved = starved
frames-dropped = dropped
frames-late = late
frames-xruns = xruns

## The song under the frame meter
#
# A developer's overlay like the block above it, and translated for that block's reason. What is
# drawn here is what the machine did to this file rather than what the file is: the title, the
# artist and the length are already in the corner above the panel.

song-heading = SONG
song-kind-midi = midi, { $tracks } tracks
song-kind-video = video
song-kind-cdg = mp3+g
song-kind-ultrastar = ultrastar
song-kind-lrc = lrc

# Where playback has reached, then the length. The two numbers are drawn by the machine.
song-position = position
song-gain = gain
song-levelled = levelled
# Where the gain came from. The four are different answers to one complaint, so a song already at
# the reference and a song nobody levelled must not read the same.
song-levelled-package = pkg { $lufs } LUFS
song-levelled-events = midi { $db } dB
song-levelled-off = off
song-levelled-none = none

song-fixes = fixes
song-fixes-none = none
song-fix-bank = bank { $count }
song-fix-mute = mute { $count }

song-lyrics = lyrics
song-flavor-soft-karaoke = soft-karaoke
song-flavor-lyric-events = lyric events
song-flavor-named-text-track = text track
song-flavor-none = no words

song-damage = damage
# A track the parser stopped reading, a track the header promised and never delivered, and a note
# left sounding that had to be closed by hand.
song-damage-cut = cut { $count }
song-damage-gone = gone { $count }
song-damage-notes = notes { $count }

## The connect panel

connect-heading = Remote control
connect-local-only = Remote control is local-only
# Whoever is standing at the machine is the person who can change this, so it names the address and
# says what to change it to.
connect-local-only-detail = Listening on { $address }. Set the bind address to 0.0.0.0 to allow a phone to connect.
connect-no-network = No network
connect-no-network-detail = Connect this machine to Wi-Fi or Ethernet to use a remote.
connect-unavailable = Remote control unavailable
connect-no-address = No usable address was found.
connect-factory-pin = · PIN { $pin }
connect-other-addresses = { $count ->
    [one] { $count } other address
   *[other] { $count } other addresses
 }
# Drawn beside the code and only where the key exists — Windows and macOS. Everything else on this
# panel answers a phone; this is the line for whoever is sitting at the machine.
connect-browser-key = F11 opens the remote in a browser
# macOS, where the window server takes the bare key for Show Desktop and the press never reaches the
# machine at all.
connect-browser-key-ctrl = Ctrl+F11 opens the remote in a browser

## The number a singer is dialling

# What the keypad line says about a number that is not a song number at all — `000`.
number-invalid = invalid number
