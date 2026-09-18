# Audio

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Microphones

**Control and state only** — mic audio is mixed in hardware; the app exposes devices, levels and
effect state over the API but applies no DSP.

## Melody channel

**Detected at packaging time only**, written into the package, and only when several independent
signals agree. Otherwise no melody channel is claimed and the toggle is hidden.

## Every song plays at the level its bank renders the corpus at

**A MIDI song is levelled too, from an estimate the machine reads out of its own events as it starts,
and it is the one kind of song that may be turned *up*.** Video and MP3+G are measured when their
package is built and brought down to the same place. All three therefore sit at one level, and that
level is the bank's own.

**The complaint is a quiet song, and nothing that only attenuates can answer it.** Files that sound
right and play low are the ones a room notices, and levelling by attenuation closes a spread by
bringing everything down to the quietest. Measured over 989 corpus songs through the recommended bank,
**56 of them play more than 9 LU below the bank's mean and average 12.1 dB down**; reaching a flat
catalog by attenuation alone would cost 8.2 dB of level everywhere.

**The spread is 12.8 LU between the tenth and ninetieth percentile**, over a 37 LU range. The
`spread` column in `soundfont-banks.conf` reports 7.4 for that bank and is seven songs: it ranks banks
and does not say what a room hears. So this is the same defect as the media gap in a larger size, and
in the kind of song the machine plays most.

**A quiet MIDI song has the headroom to come up, and that is a property of the corpus rather than a
hope.** 98% or more of the songs in every band below the mean have two to three times the room they
need under −1 dBTP, because a quiet MIDI song is sparse or lightly played rather than compressed, so
its peak falls with its loudness. A media song is a finished master and has no such room, which is why
[`Video and MP3+G play at the MIDI reference level`](#video-and-mp3g-play-at-the-midi-reference-level)
still only attenuates. **Two kinds of song, two rules, one target.**

**Levelling clips less than not levelling.** Ten percent of the corpus already passes −1 dBTP on the
recommended bank at `music_volume: 1.0`, the worst at +7.2 dBTP. Applying the estimate leaves 2% of
songs marginally past it, worst +2.5, because the rule that raises a quiet song lowers a loud one. The
comparison that decides this is against the machine as it stands, not against silence.

**The level is read from the events, with no synthesizer and no bank.** It costs 0.19 ms a song and
1.6 ms at worst, on a song the machine has already parsed, and it lands within 2.3 LU of what a render
through the recommended bank says. The alternative was measuring at packaging time, and it was
rejected on two measurements rather than on taste:

- **Rendering through a bank that is not the one playing the song is worth no more than the estimate.**
  Across six banks and fifteen pairs the residual runs 1.39 to 3.81 LU, median about 2.6, against the
  estimate's 2.55. A bank that flattens the difference between instruments flattens the difference
  between songs: the fit from the recommended bank to `aspirin` has a slope of 0.657.
- **A packager has no bank.** `tools/dist/cmd.sh` stages each tool as its executable and a README, and
  says in place that there is no asset-fetching step because none of them carries a SoundFont. A rule
  that needed one would work for the person who built the machine and for nobody else.

**Estimating at load rather than at packaging is a departure from the melody channel's rule, and the
reason that rule exists does not reach here.** Detection and media metering are at packaging time
because a full audio decode is seconds a person would wait for. This is a fifth of a millisecond on a
parse that already happens, and taking it at load buys what a package never could: **every song
already installed is levelled without being rebuilt, and so is a file played straight from disk**.

**The bank cancels out of the arithmetic, so nothing looks a reference up.** A song's estimate predicts
its level as `bank mean + (estimate − corpus mean)` and the target is the bank's mean, so the gain is
the song's distance from the corpus and nothing else. `km_loudness::MIDI_REFERENCE_ESTIMATE` is that
corpus figure, and it is the only constant a change to the estimating code has to move.

**Raised by at most 12 dB.** The songs this exists for average 12.1 dB down, so a smaller cap would
leave them half fixed; the cap is what stops a wrong estimate becoming an arbitrary multiplier in the
audio callback. `Player::set_song_gain` clamps to the same figure, and a media song is held to 1.0 by
`gain_for` rather than by that clamp.

**It makes some songs worse, and the size of that is measured.** The estimate cannot see which
instrument a program number selects, so its error is 2.3 LU. Over the corpus the mean distance from the
target falls from 3.99 LU to 1.68, **78% of songs move closer and 11% move more than 1 LU further
away**, worst case 6.8 LU. A rule that is right on average and wrong on one song in nine is the trade,
and the alternative on offer was being wrong by 12.8 LU by design.

**`audio.normalize_midi` turns it off, and it is a second key rather than a share of
`audio.normalize_media`.** They are two decisions about two kinds of song, and an owner who has
balanced a room around unlevelled media may still want quiet MIDI files brought up. This is also the
only levelling that can make a song louder, so it needs a way back that does not take the other with
it. Like the key beside it, it is not on the API.

**Nothing is stored and nothing is measured at packaging time**, so `km-pack`, the manifest and the
catalog are untouched. Rendering through a named bank would beat the estimate outright, and the place
it would earn that is a package built for one machine whose bank is known; that is a change to make
when somebody wants it, against the same gain.

**A file played straight from disk is levelled if it is MIDI and not if it is media**, and the
asymmetry is the price of reading a MIDI level at load. A media song's level is a full audio decode,
about 0.6 s, where a MIDI song's is 0.19 ms on a file the machine has already parsed; one of those
belongs in front of a song starting and the other does not. So `play_video_path` and the MP3+G path
beside it carry no measurement, exactly as
[`Video and MP3+G play at the MIDI reference level`](#video-and-mp3g-play-at-the-midi-reference-level)
says a song without one behaves.

**What that costs is that an audition does not sound like the package built from it**, and
auditioning is how a curator judges a file. A media song from disk plays at whatever it was mastered
at, and across a real library of 286 karaoke files that is a median **−14.2 LUFS for video and −14.9
for MP3+G**, against the −22.8 a MIDI song is now brought to: **about 8 dB**. Anybody comparing the
two kinds by auditioning them — the obvious thing to do — hears a gap the real catalog does not have.
**Judge media against MIDI from a package**, and read the machine's own `debug!` line to tell them
apart: a levelled song logs one and an unmeasured song logs nothing.

The corpus figures behind every number here are in
[`docs/research/midi-loudness.md`](../research/midi-loudness.md).

## Video and MP3+G play at the MIDI reference level

**Each video and MP3+G song is measured when its package is built, and the machine attenuates it at
playback to the loudness its SoundFont bank renders the MIDI corpus at.** A media song carries a
measurement and only ever comes down; a MIDI song is brought to that same level by the entry above,
either way, from its own events.

**The complaint was that a MIDI song is quieter than either, and that the two media kinds differ from
each other, and both halves are worse than they sound.** Measured on real corpus files against the bundled
bank's −21.9 LUFS: a karaoke video at −17.1 LUFS and an MP3+G pair at **−5.5** LUFS. So one machine at
one setting of one amplifier plays those three 4.8 dB and **16.4 dB** apart, and the two media songs
11.6 dB apart from each other. That is not a preference anybody can dial out — the gap moves with
every file, because it is whatever its publisher mastered to.

**It is the same argument the bundled bank was chosen on, one layer out.** That choice was decided on
song-to-song spread rather than on any one song's sound, because *"a karaoke machine has its
music-to-microphone balance set once, in hardware, and then plays a hundred different songs at it"*
([`docs/architecture/audio.md`](../architecture/audio.md)). A 16 dB step between kinds is that defect
in a larger size, and no bank can fix it: it is not in the rendering, it is in the files.

**Attenuation only for media, and that is forced rather than chosen.** A commercially mastered file
has no headroom to give, and `rustysynth`'s master volume is deliberately left at its hardcoded 0.5,
because raising *that* would invalidate every loudness figure the bundled bank was chosen against. So
a media song comes down and nothing raises it. A MIDI song is the case where the headroom is measured
and present, which is what the entry above turns on.

**The reference follows the bank rather than being a constant.** The fifteen banks measured span
**−12.4 to −26.6 LUFS**, so a machine on `somsak` renders MIDI as loud as a karaoke MP3 and one on
`sc55` renders it 14 dB quieter; a fixed target would be wrong by that much the moment anybody chose a
different bank. `soundfont-banks.conf` carries a `lufs` per row, measured at `music_volume: 1.0` so
the owner's own level cancels out of the comparison — it multiplies a MIDI song and a media song
alike. A bank nobody has measured falls back to a figure in the right region: **abstaining is not
neutral here**, because it would mean no levelling at all, which is the complaint.

**Measured at packaging time, like the melody channel, and for the same reason** — it is a fact about
the bytes, so a machine should read it rather than derive it while somebody is waiting. A video's
audio is measured **as it will be stored**, not as it arrived: the packaging profile's `-ac 2` turns a
mono source into stereo, which moves the measurement about 3 dB by itself.

**A media song with no measurement is left alone.** That covers every package built before this and
a media file played straight from disk with no package behind it; a MIDI song is levelled from its
own events instead, by the entry above. `km-pack reanalyze` measures the media *inside* an existing
package and rewrites only the manifest, so nobody has to keep the sources to gain it — the
re-analysed package is the same size as a freshly built one and carries the same numbers.

**A package carrying no measurement is the ordinary case for a library built before this, and it
says nothing about it**: the songs play at the level they were mastered at, about 8 dB above where
MIDI now sits, and only the absence of a `debug!` line distinguishes that from a song the machine
decided to leave alone. `km-pack inspect --songs` says which packages carry numbers, and `reanalyze`
is what gives them some.

**`audio.normalize_media` turns it off, and it is not on the API.** It changes how every media song
sounds, so an owner who has already balanced a room around the old behaviour needs a way back that is
not rebuilding their packages. It is a statement about how a machine is set up rather than an
adjustment to a performance, which is the same standing `audio.idle_release_secs` has and the reason
the four things a guest's phone may change remain the key, the tempo, the volume and the guide melody.
Turning it off un-measures nothing: the numbers stay in the packages.

**The measurements are not on the API either**, on the judgment
[`Switching the bank while it plays`](#switching-the-bank-while-it-plays) already makes about a
debugging control: a per-song loudness is not a number anybody in the room acts on, and putting it on
a published surface is seven places to keep in step. `km-pack inspect` reports it per song with the
gain beside it, and the machine logs the applied gain at `debug!`. **The `/dev/` page does not show
it**, which was intended and is not possible as things stand: that page reads the same DTOs the
published API serves, so drawing the gain there would mean adding the field this paragraph declines.

## Lyric timing offset

**A display-side offset in milliseconds, `display.lyric_offset_ms`, positive meaning the lyrics lead
the audio.** Clamped to ±500 ms, live over `PUT /api/v1/settings`, persisted, and deliberately *not*
reset between songs — it calibrates a room, not a performance. It shifts only the tick the local
display draws from; the engine, the audio and the `lyric_line` API event are untouched.

**Delaying the audio to match late video is what an AV receiver's lip-sync control does, and it is
unusable here**: with microphones mixed in hardware, the singers' own voices are in the signal it
would delay, so lip-sync would put the room's own singing behind the music.

## Guide melody default

**On.** The people this machine is for are singing songs they half know, and the guide melody is what
makes those singable at all; starting it off means whoever needs it most has to know it exists before
they can find it. Anybody who does not want it turns it off in one press from the display or one tap
from a remote.

It costs nothing where it cannot help: the melody only sounds when **detection was confident**, which
is a high bar by the row above, so a file that abstained plays exactly as it would have. This is a
*default*, not a policy — the toggle is mirrored into the stored settings, so a machine whose owner
has turned it off stays off.

## Holding the audio device

**The machine holds the output device only while it has something to play.** It is opened on the first
thing that means sound — a song queued, loaded, played, restarted or sought — and handed back five
seconds after the last one ends.

Holding it for the life of the process is free on a box under a television whose speakers nothing else
uses, and on a laptop with **Bluetooth headphones** it is enough to stop every other application
making a sound from the moment the app launches. Nothing exclusive is involved — the output path is
shared-mode throughout — and merely *occupying* the endpoint is the fault.

**The device is dropped, not paused**: pausing stops the clock and keeps the endpoint, so it would fix
nothing. `audio.idle_release_secs` sets the delay, and **`0` never releases it**, which is the right
setting for a dedicated machine.

The cost is that the first song after a quiet spell starts up to a second late on Bluetooth. A *late*
start, not a clipped one: song time only advances while the device's callback runs, so the lyrics
arrive with the music rather than a second ahead of it. Queuing a song sends the hint that starts the
device coming up, so most of that second is spent while somebody is still choosing.

## The machine sleeps when it leaves the screen

**A machine that is off the screen lets the system take its audio device, and has it back on return
with the paused song still loaded at its position.** The screen is the product, so a machine nobody
can see is a machine nobody is singing on, and the battery is what it costs to pretend otherwise.

**This is the system's job rather than the machine's, and the machine's part is to stay out of the
way.** iOS interrupts an audio session when it suspends an application, and cpal's iOS backend
listens for that interruption and stops and resumes the output unit on it. An application that
deactivates its *own* session posts no interruption to itself: the unit then runs on against a
session that is gone, and the transport answers every request while advancing nothing. So the machine
declares no background-audio mode, and lets the suspend it would otherwise have blocked happen.

**Holding the device off the screen costs about three points of battery an hour**, measured on an
iPad with the screen off and the charger out: 4.9 %/h with a song paused and the device held, against
1.6 %/h suspended, which is twenty hours to flat rather than sixty. A paused song renders silence,
and the system cannot tell silence from music at the callback.

This sits beside [`Holding the audio device`](#holding-the-audio-device) rather than replacing it.
That rule is about an idle machine on a desk, and its five-second release still governs one; this is
about a machine that has gone away, where the transport may be `Paused` and the device goes back
anyway.

## Choosing the audio output device

**The machine names its own output, by an identifier that survives a reboot, and remembers it.** On
the real appliance the **ALSA card order moves between boots**, and the whole audio design routes
one specific interface into a hardware mixer the microphones also feed. A machine that relocates its
output to HDMI after a power cut is mute in front of a room, with `RUNNING` in `/proc/asound`, a
healthy service and nothing wrong in its log.

**An identifier, not a hint.** `cpal` gives every device a `DeviceId` documented as stable across
restarts, with `Display`/`FromStr` and a `device_by_id` lookup, so **one opaque string is the whole
setting** and there is no name-matching to get wrong.

`audio.output_device` has three states that are deliberately not two: **absent** means nothing has
been chosen, **`"system"`** means follow the system default *deliberately*, and anything else is a
device. **A saved device that is not present falls back to the system default and the setting is left
alone**, so an interface unplugged for one evening is used again the moment it returns, and the API
reports `selected` and `active_id` disagreeing rather than pretending.

**On Linux the machine prefers a USB interface, on every start, and never writes down what it
picked.** A guess that is written down stops being a guess and starts being a decision nobody made:
recording it means that from the second boot the machine is no longer preferring USB but naming one
concrete identifier — precisely the behavior the preference exists to avoid — and if the interface is
missing for the one boot that counts, what gets recorded is `"system"`, which means *chosen
deliberately*, silently switching the preference off for good on exactly the machines that need it.
So `decide` runs at every start and `audio.output_device` holds only what a person put there.

**Linux only.** cpal's `InterfaceType::Usb` would carry this to Windows and macOS and is deliberately
not taken there: those are desktops where the system default is right and a plugged-in interface is
not a request to move the sound. cpal cannot help on Linux either — that field is populated by WASAPI
and Android and never by ALSA — so `/proc/asound/cards` is read for the driver field, and the stable
`CARD=<name>` spelling is preferred over `CARD=<index>`, which is the number that moves.

Changing it is **`PUT /api/v1/admin/audio/output`, refused with a 409 while anything is playing or
queued**: the player lives inside the audio stream a change has to drop. `--list-audio-devices`
prints the identifiers, because on a box under a television there is no browser to find one with. It
says the machine picks again every start rather than naming a choice, because nothing was chosen.

**Two pages draw the picker**: `/admin/`'s Sound tab and `km-admin`'s Sound page, which are one place
under one name because
[`Two admin surfaces, one vocabulary`](distribution.md#two-admin-surfaces-one-vocabulary) asks for
it, and on **Sound** rather than *This machine* because it is about sound.

**Everything decided above had to be drawn, and each half shows up as a rule on the page.** The
sentinel leads, worded as *follow the system* rather than by its identifier. Only the `preferred`
rows are offered, with every spelling behind `?all=1` — a query parameter and not a checkbox,
because `/admin/` carries no script. Whatever is selected is always shown whatever its spelling. A
saved device that is gone is listed as not plugged in rather than hidden, because the setting is
deliberately left alone and hiding it would report the fallback as though somebody had chosen it.
And the fallback itself is drawn as a warning beside the picker, because after one the setting and
the sound disagree and a page showing only the `<select>` shows the choice without the consequence.

**The picker is drawn even while the machine would refuse it**, with a line saying to stop the music
first. `A control that can only be refused is left out, not grayed` names `AudioOutputs::changeable`
as the precedent for *having* the flag and not for spending it: a device that cannot be changed now
can be changed when the song ends, so leaving the control out would be taking it away for the length
of a song.

**`AudioOutput::system_default` does not mean *this row is the sentinel*, and the API's own summary
of it said that it did.** It marks the real device *follow the system* resolves to today, so
somebody choosing the sentinel can see what they are choosing; the sentinel is identified by its
`id`. The picker was written against the wrong sentence and labelled the onboard card "Follow the
system" while the real sentinel showed the audio backend's untranslated English. `km-api` gained
`machine::SYSTEM_OUTPUT` so a page can ask the question without `km-api` depending on the audio
backend, and `karaokemachine` — the one crate that sees both — holds the two spellings equal in a
test.

**The list a person is shown is the operating system's device list, not the backend's — one row per
physical output.** ALSA hands over its *configuration* rather than its hardware: the kernel's
enumeration is concise, one entry per card and device, and alsa-lib layers channel maps (`front:`,
`surround51:`), plugin chains (`sysdefault:`, `dmix:`), numbered aliases (`hdmi:`, `iec958:`) and
routing PCMs (`default`, `pulse`, `null`) on top, cpal enumerates every one and repeats the hardware
under the card *index* as well as its name, and each is described with the *card's* own words. One
jack arrives ten times under one string.

**The structural rule is that `hw:` and `plughw:` are the kernel's devices and everything else is
configuration over one of them** — grouping by display name works and infers hardware from a
description. The row offered is the same spelling the first-run rule would choose, so choosing by hand
and choosing by rule cannot disagree.

**Nothing is hidden from the machine, only from the first screen**: every spelling is still sent
carrying a `preferred` flag, still accepted by `PUT /audio/output`, and reachable behind "show every
device". A saved identifier is always shown whatever its spelling, because a choice somebody has to go
looking for is a choice they cannot change. `alsa:default` is **not** offered, because "follow the
system" is what the sentinel means and it means it with an identifier alsa-lib cannot reconfigure out
from under it.

## The machine sets the level its output runs at

**The level in the operating system's own mixer is the machine's to read and to move, and it is
reported in decibels.** It sits between the synthesizer and the amplifier, downstream of
`music_volume`, of a song's own gain and of the limiter alike, and nothing in the audio callback
reaches it.

**A percentage on such a control is a position on its range, not a loudness, which is the whole
reason the unit is decibels.** A control spanning `0 - 128` steps with a floor of −128 dB puts
−20 dB at "84 %" — a reading that says *almost all the way up* about a tenth of the voltage. The API
carries `AudioOutputs::level` in decibels and a page prints the figure `amixer` prints, so a number
read on a screen and a number read in a terminal are the same number.

**Installation configuration, like the device it belongs to.** It is set once when a room is
balanced, so it lives under `/api/v1/admin/`, it is absent from the settings DTO, and nothing about
it rides the state broadcast. The four things a guest's phone may change remain the key, the tempo,
the volume and the guide melody.

**The operating system is where it is remembered, and the machine stores nothing.** ALSA restores it
from `/var/lib/alsa/asound.state`, Windows keeps an endpoint's volume itself, macOS keeps a device's
in its own preferences, and a machine keeping a second copy would overwrite at every start whatever
had been set from anywhere else. What a clean
shutdown does not survive is a power cut, which costs one slider.

**Not refused while a song plays**, unlike the device: the level belongs to the sound card rather
than to the audio stream, so moving it disturbs nothing and there is no `changeable` to consult.

**An output may have no level, and that is an answer rather than a fault.** HDMI and S/PDIF carry
samples to a receiver that holds the volume, so the card offers a switch where an analog path offers
a knob. `level` is then absent and the control is **left out rather than drawn dead** — the other
side of the bargain `changeable` makes, because a device that cannot be changed now can be changed
when the song ends and an HDMI output never gains a level.

**Playing through a sound server is the second case with no level**, and for the same reason rather
than a different one: `default`, `pulse` and the rest are alsa-lib configuration over a card, not a
card, so there is no hardware control behind them to move. The machine says there is none instead of
moving something the sound server will override.

**"Follow the system" is resolved to whatever it is today.** It is what the engine reports as active
whenever nothing has been chosen by hand, which is every machine out of the box — so a level read
from the sentinel itself would make this a feature only a machine with a hand-picked output had.

**The route is mounted only where the host has a mixer to reach**, the shape
[`Power is a capability of the host, not a method on the machine`](api-and-network.md#power-is-a-capability-of-the-host-not-a-method-on-the-machine)
prescribes: Linux, Windows and macOS answer, and a host with nothing to ask answers 404 rather than
carrying a route that could only refuse. Android and iOS are the hosts with nothing to ask, an
application on either being unable to move the system's own volume at all.

**Where the operating system knows what has a level, it is asked rather than reasoned about.**
CoreAudio carries the answer as a property of the device, so on macOS an HDMI output, an aggregate
device and an AirPlay speaker each say for themselves whether they attenuate, and the two paragraphs
below about control names and digital carriers are ALSA's alone. Inferring it is what a platform
carrying no such property leaves.

**A device that reports a position and not decibels is still reported in decibels.** Some outputs
carry only a place on their range, and the platform's own conversion of one to the other is linear
over the range the device reports, so that is what the machine reports too. What it will not do is
print the position: `43 %` of a −40 dB range is −22.8 dB, and only one of those two numbers says how
loud the output is.

**A decibel reading that does not move with the control is not one.** A Bluetooth headset answers
every decibel read with unity while its own position sits at 43 per cent, so the reading is checked
against the position before it is trusted and the position is used where the two disagree. The check
is worth its three reads because the failure it catches is a page saying an output is wide open
while the room hears it 20 dB down, which is the fault this whole control exists to make visible.

**Which control governs an output is a decision, not a lookup.** A card with an analog jack and three
HDMI outputs offers one `Master` between them, and it governs the jack — so a digital output is
refused before any control is examined, and an analog one takes `Master` before `PCM` because a card
offering both puts the output stage on the former. A USB interface typically offers `PCM` alone.

**The conventional names are a preference and not a whitelist.** ALSA control names come from the
driver and there is no registry of them, so a card that names its output something unanticipated
takes its first playback control that is not an input being monitored. A whitelist would answer
*this output has no level* on the first unfamiliar card, which is wrong in the only direction that
cannot be noticed: it reads exactly like an HDMI output, and an owner would believe the hardware
rather than the machine. What the fallback refuses is a control whose name says it governs a
microphone, a line input or the PC speaker, each of which carries a playback level without being the
output.

**A rise of more than 6 dB is asked about on a page, and the route asks nothing.** Six decibels is a
doubling of voltage, and this is the gain into an amplifier somebody has balanced a room around; the
accident worth one extra press is a drag from one end of the slider to the other rather than a nudge.
Lowering never asks, being audible, free and undone by the same control. `Removing a bank` above
draws the same line: pages ask first and the route does not.

**The reading is drawn where the control is hidden.** The figure is what makes a quiet machine
diagnosable, so it is always on the page; the slider sits behind a link, which on a page with no
script is what a disclosure is.

**Three surfaces carry it** — `/admin/`, `km-admin` and `/dev/` — which
[`Two admin surfaces, one vocabulary`](distribution.md#two-admin-surfaces-one-vocabulary) requires of
the first two. The console has neither the link nor the question: its whole surface needs no
password by design, and a guard against a careless drag is not what a debugging page is for.

## A song's own defects are corrected at playback, and only from what the file says

**A fix is a filter on a song's MIDI event stream, decided when the song is analyzed and applied
every time it plays.** The corpus is full of files written for a synthesizer module that is not the
one playing them, and some of those files are unpleasant rather than merely inaccurate: a setup
track that sends Bank Select MSB 127 on a melodic channel is asking for XG's drum bank, so on a
SoundFont that has one the channel stops being an instrument and its sustained chords come out as
agogos, maracas and bells. `km-fixes` is where corrections of that kind live.

**A fix may depend only on what the file says, never on how a bank renders it.** A package is built
once and played on machines with different SoundFonts, so bank knowledge is not available at the
moment a fix is decided — and the defect above is inaudible on a bank with no bank 127, which falls
back and plays something wrong but harmless. **A defect in how one bank renders a correct file is
not a fix**: that belongs to `km-banks`, which already carries per-bank measured data, or to a
machine setting.

**A fix that makes every bank agree may apply itself. A fix that picks a winner must be offered.**
Ignoring a stray bank select gives the same result everywhere, because the fallback a font without
that bank already performs becomes what all of them do. Silencing a channel deletes music some banks
render correctly, so it stays off until a person turns it on. The two halves are one rule and not
two policies: what separates them is whether anything is lost that somebody might have wanted.

**A part that spoils a song can be silenced or re-voiced, and both wait for a person.** Silencing
deletes the part; a forced instrument keeps the notes and plays them on something else, which is the
gentler answer where the arrangement needs the line. Neither has a detector and neither can have one:
what makes an instrument wrong is how the bank in the machine renders the program the file asks for,
and that is a fact about a bank rather than about a file. **The rule that a fix may depend only on
what the file says governs the detector and not the person.** A curator listening to the song is
hearing the bank, so they may say what no rule may propose.

**A bend left off centre is suggested, and waits for a person.** A part that bends and ramps back can
stop one step short of centre, and every later note on the channel then plays out of tune until the
file bends again. Every bank agrees about that, so recentring it does not make banks agree; it
changes what the file asks for. Most of the time that is the repair. Sometimes a part means to play
into a held bend, and nothing in the file tells the two apart with certainty. So the detector
suggests the channel, the curation tool offers it unticked, and the machine recentres the bend only
where somebody ticked it.

Measured over every twenty-fourth file of the corpus (2026-09-17): of those that parsed, **0.24%
have a channel the detector suggests a recentre for**. A bend held at full deflection is not among
them: a held bend is common and deliberate, and the detector suggests only a return that stopped
short.

**The drum channel is offered no instrument.** A program change there selects a kit, so a melodic
program on it names nothing a singer would want.

**Detected and edited both, which no other field in a manifest is.** A transposition is only ever
typed and a melody channel is only ever found. A fix list is proposed by a detector and then possibly
decided on by a person, so a rebuild re-derives it — which is what carries a newly recognised defect
into a package built before the detector knew it — unless `EditedField::Fixes` says somebody has
answered, in which case their list is carried whole. **An empty list is one of those answers**, and
the only way to refuse a fix that would otherwise apply itself.

**What the machine plays is the stored list, entire.** Filtering it at playback would re-decide a
question the package already answered, and the first thing it would throw away is the channel mute
somebody set by hand.

**The list is bounded by what `km_song::EventKind` can express.** `Song` carries no SysEx, so no GM,
GS or XG reset can be detected or corrected, and nothing here touches lyrics, sync or missing notes —
those stay problems with a file rather than things a machine quietly works around.

Measured over every twenty-fourth file of the corpus (2026-09-11): of those that
parsed, **1.31% carry a kit bank select on a melodic channel**.

**That figure comes from parsing, and searching the bytes for the pattern gives a different one.** A
byte search finds `Bn 00 7F` wherever it sits, including inside a track name, a delta time or a
lyric, and it misses every such message written under running status, which has no status byte to
match. Over the same corpus the two errors did not cancel: the byte search reported 1.20%, so the
messages it cannot see outnumber the coincidences it counts.

## The synthesizer is a fork

**`rustysynth` comes from a git branch rather than from crates.io**, and it is the only dependency in
the workspace that does. Two faults in the published 1.3.6 are the reason, and both are ones this
machine walks into rather than ones it might.

The first is that it reads the `pmod` and `imod` chunks and throws them away, so **no SF2 modulator is
heard at all** — which is not neutral between banks, since the one whose design leans hardest on
modulators is the one it ignores hardest. The second is that **one defective
record refuses a whole bank**: four of fifteen banks surveyed would not load, and in one case the defect
is exactly one `shdr` out of 5,007 costing 1,611 MiB. That is also why `--set-soundfont` opens a bank
before writing the key.

The fork's 1.4.0 implements the modulators; its 1.5.0 drops the bad record instead of the bank. A
wholly unplayable file still fails, which is the part that must not change — the check exists to stop
the synthesizer panicking later.

**The pitch path reaches this machine too**, because `km-audio`'s sink forwards *every* controller, so
an RPN or NRPN sequence in a karaoke file arrives as four ordinary control changes and is acted on:

- **Roland GS drum pitch (NRPN 18H) is honored**, per key, on a channel holding a drum kit. A
  channel-wide tune cannot express it, because on a drum part each key is a separate instrument. On
  the file it was found in, dropping it left 915 of 1,982 percussion notes at the wrong pitch.
- **The RPN/NRPN selectors and both data entries are masked to seven bits.** The damage was lasting
  rather than momentary: 255 into CC 6 under RPN 1 detuned the channel for the rest of the file.
- **`scaleTuning` does not scale pitch bend, vibrato or channel tune** — SF2 2.04 8.1.2 defines it
  as the influence of *key number* on pitch, so scaling the modulation by it leaves a fixed-pitch
  region deaf to the pitch wheel. GeneralUser GS ships eight such regions and SGM-V2.01 ships 174; a
  region at the default 100 is unaffected, which is nearly all of them.
- **A seek re-establishes the parameters themselves, never their data entries.** The six controllers
  that make one up carry no value between them: CC 6 is a bare number, and which parameter it lands
  on is whatever the four selectors chose last. So the seek replay states each parameter with its own
  selector and then puts back the selector state the file left, where every other controller is
  replayed by number with its last value. Replaying these by number too would put the data entry
  first, where it names the null parameter and is discarded in silence — a file asking for a
  twelve-semitone bend range would play every bend at two, which is a part out of tune rather than a
  part missing, for the rest of the song and in every key.
- **The GS drum tune is tracked for the drum channel alone** across a seek, at one slot per key. The
  message that moves a kit to another channel is SysEx, which `km_song::Song` does not carry, so
  channel 9 is the only kit the synthesizer can hold — and 128 slots for one channel is what the 4
  KiB for sixteen would otherwise cost on a thread that may not allocate.

**The hold pedal reaches it too, and is the one of these that costs a whole part.** CC64 defers every
note-off on its channel until the pedal is observed up, and the observation happens once per render
block — so a file that lifts the pedal and presses it again on one tick is never seen to lift, and
the voices it deferred sound on to the end. The fork counts lifts on the channel rather than reading
the pedal's position, which is what makes a lift inside a block observable. See
[`Nothing is left sounding`](#nothing-is-left-sounding).

The fork's corpus check puts the cost at 8 files of 150 rendering differently. Its MIDI *parsing*
fixes reach nothing here: `km-song` parses with `midly` and this workspace never constructs a
`MidiFile`.

**A git dependency is a source, not a package.** The branch is `custom`, `Cargo.lock` pins the exact
commit, and every release path passes `--locked`, so a build is as reproducible as against the
registry. `branch` rather than `rev` states that the fork is still moving; it becomes a `rev` when
it stops. **It moves more than that reads**: the branch can advance twice in an afternoon, and cargo
takes the newer commit the next time it re-resolves. So a measurement session must record which
commit it measured, and re-resolving is a deliberate act with a diff to read. **The exit is a
version number**: the day upstream carries them all, the line goes back to `rustysynth = "1.x"`.

**What the modulators cost the bundled bank, measured.** The seven songs of the research note, 90 s
each at `music_volume: 1.0`, put it at **7.8 LU** of song-to-song spread — the one number this project
decided that bank on, and 1.1 LU wider than the same bank rendered with the modulators discarded.
Polyphony is not the variable: 256 and 64 reproduce each other to the decimal.

**A bank moves under modulators if and only if its author wrote preset-level ones.** GeneralUser GS
has 1,194 `pmod`; SGM has 119 and is 0.5 LU *narrower* with them honored; the banks with none are
unchanged to the decimal.

**Scaling the reverb and chorus sends back does not help.** 1.4.0's default modulators use reverb
and chorus send amount 1000 where SF2 says 200, and `reverb_send_scale` scales that back; at 0.2 the
bundled bank stays at 7.8 LU exactly and MuseScore gets worse. `km-audio` sets neither scale.

**Leniency reaches four banks in the survey** — the widest of them, 891 presets, is refused over
**two** empty loops; others drop 6, 14 and 15 records. Two files are refused at the container rather
than at a record and should be: an SF3's Ogg `smpl`, and a `sfpk` RIFF form.

**None of that reopens the bundled bank.** The best of them is 261 MiB against a 30.9 MiB shipped
tree, so the size gate refuses it before sound is discussed, and it has no pinnable source, so it
cannot be a documented override either. **SF3 support is still the single change that would reopen the
choice.**

## A partly loaded bank plays, and says so

**A bank that lost records to load is played, not refused — and every surface that names a bank
names what it lost.** [The fork](#the-synthesizer-is-a-fork) drops a defective record instead of the
whole file, and the cost of that is a bank that is quietly *incomplete*. An instrument whose regions
were dropped does not sound, and nothing about the file, the log or the settings key would say why.

**Refusing it is the wrong answer.** It would throw away the point of loading leniently: a bank
losing two records out of 891 presets is 889 working presets worth having, and for a machine that
falls back to a sine test tone, no bank at all is plainly worse.

**So it is said in the two places somebody can act on it.** `--set-soundfont` prints it, because that
is the moment a person is choosing a bank; and the machine logs it at `warn!` rather than `info!` at
startup, on the rule already in that code — the other two audio states are warnings because the owner
needs to know about them. One line in both: the count, the first three records in the synthesizer's
own words, and how many more there were.

**It is not on the API.** `GET /audio/soundfont` carries a `problem` field meaning *why there is no
bank*, and a bank that is playing is not that — a client rendering `problem` would report a working
machine as broken. `--show-paths` does not say it either, because it never loads the bank.

**The count and the examples come from two different calls.** The synthesizer retains the first 64
warnings and counts every one, so "37 dropped" and "here are three of them" are not the same number's
beginning and end; `BankDefects` keeps both and computes the remainder from the total, so a bank with
500 defects says "and 497 more".

## Aftertouch: channel pressure yes, per-note pressure no

**Channel pressure is forwarded to the synthesizer and replayed on a seek; polyphonic key pressure is
dropped.**

**Measured, because "aftertouch is rare in karaoke files" was an assumption worth checking**: over
25,000 corpus files, `cargo run --release -p km-song --example event_census` reports channel pressure
in **7.66%** of them and 1.6 million events, against polyphonic pressure's **0.67%** and 43,000. One
file in thirteen is not rare, and where it appears there are hundreds of events.

**Poly pressure is dropped for a reason that has nothing to do with rarity.** It addresses a *note*,
and this sequencer transposes notes — so forwarding it correctly means mapping its key through the
sounding-note table exactly as note-off does, and forwarding it incorrectly means applying pressure to
a note nobody is holding. That is most of the work and, at 0.67% of files, almost none of the benefit.
Revisit it with the transposition, not without.

**Channel pressure is replayed on a seek**, with the controllers, programs and pitch bends, because it
is channel *state* that persists until something changes it. A jump past the point where a file leant
on it would otherwise play the remainder unpressed — the same defect as seeking past a program change
and landing on a piano.

## Choosing a bank

**`audio.soundfont` holds a bank id, and the `/dev/` page lists every bank on the machine and switches
to one mid-song, kept across restarts.**

The banks come from `data_dir/soundfonts/`, a folder beside `packages/` and `wallpapers/` that
`Paths::create` makes and `--show-paths` names: a place the owner is *meant* to find. Deliberately not
under `assets/`, which ships with the build, is read-only where it matters, and is copied wholesale
into every carrier — `tools/dist/check-assets.sh` refuses a staging run where `assets/soundfont/`
holds two banks, and this folder is where the second is allowed to live.

**On Android it is two folders, and the second is the one anybody can write to.** `data_dir` there is
the app's *private* directory, which a file manager, a USB copy and `adb push` all cannot reach
without `run-as`. `Paths::soundfonts_dirs` adds `getExternalFilesDir(null)`, exactly as
`packages_dirs` does for songs, with the same collision rule: **the private folder is scanned first**,
so a copy dropped onto shared storage cannot displace a bank the machine downloaded. `--show-paths`
prints every folder scanned rather than the first.

**Only nine banks are offered for download** (`Which banks the machine offers` in
[`repository.md`](repository.md)); everything else in the table is for testing, and testing means
putting fifty files on a device by hand.

**The list says what the setting names, not what is sounding**, and those differ during an A/B, where
a debug slot has swapped the bank for this run without touching the setting. `GET /audio/soundfont`
remains the answer to "what is playing".

**Behind the admin password — the same reasoning as the output device.** A guest phone
changing key, tempo or volume is the design; a guest phone rebuilding the audio stream in the middle
of somebody's song is not. Like every `admin` mark it is dormant until a password is set.

**Mid-song rather than at the next song**, accepting the audible hole described below: somebody
choosing a bank is listening to the one they have, and a change they cannot hear for three minutes is
one they cannot judge.

**Ids are slugged from the filename**, because real bank files are `Roland SC-55 v3.7.sf2` and the
alternative is `%20` in front of anybody reading a URL for the rest of the feature's life. They do not
round-trip to a path: choosing looks the id up in the list rather than rebuilding a filename, which is
what stops a crafted id reaching the filesystem.

**The machine fetches a bank when somebody asks for one by name** — see `Nothing downloads` in
[`repository.md`](repository.md#nothing-downloads) and `Where a bank may be fetched from` for the
terms. A bank arrives in that folder because somebody asked for it, one at a time, by name. Nothing is
fetched unasked, and nothing is fetched for a song.

## Removing a bank

**A bank can be deleted from the machine that downloaded it.**
`DELETE /api/v1/admin/audio/soundfonts/{id}`, answering the same bank list
[`Choosing a bank`](#choosing-a-bank) does, so a page redraws from one answer rather than deleting
and then asking what is left.

**Everywhere but Android this would not need to exist**: a bank is a file in a folder `--show-paths`
names, and removing one is somebody's file manager. **On Android that argument fails completely**, and
it fails on the machine this product is most trying to be. The downloader writes to app-*private*
storage — no file manager, no USB copy and no `adb push` reaches it without `run-as`, and a box under
a television has no shell anywhere near it. The offered banks run to 301 MiB and the recommended one
is 262 MiB, so one mistaken tap on a remote was permanent until the application was uninstalled.

**The plural is the path.** `/audio/soundfont` is the bank in force and `/audio/soundfonts` is the
collection; deleting a member of a collection belongs on the collection, as `DELETE /packages/{id}`
already does. Hanging `{id}` off the singular would put it beside the static `fetch`, where a static
segment wins the match: a bank whose file slugged to `fetch` could then never be deleted, and the
failure would be a 405 from the wrong route.

**Deleting the bank the setting names is allowed, and falls back to the bundled one.** Refusing until
something else is chosen is tidier and wrong for the case the route exists for: the selected bank may
*be* the mistake somebody is undoing, and a refusal is a dead end for somebody holding a D-pad. The
fallback goes through the same selection path as choosing the bundled bank by hand, so the level
protocol is the existing one. **The file is removed last**: a fallback that fails leaves nothing to
undo, where deleting first would leave `audio.soundfont` naming a file that is gone.

**Four refusals, each for its own reason.** An id not in the list, resolved through the installed-bank
list rather than rebuilt into a path. The bundled bank, which is unpacked from the build and would
return at the next start. **While a download is running**, so a delete cannot race the rename that
finishes one. And a bank named by a `debug.soundfonts` slot — those are slots an owner configured by
hand, and a route that deleted what one names would make the switcher's own keys the thing that broke.
See `Only debug. names a file` in [`foundations.md`](foundations.md).

**The pages ask first, and the route does not.** `/admin/`'s Sound tab sends Remove to a confirmation
page naming the bank and its size; `/dev/` asks with a `confirm()`. See
[`A page asks before it deletes a file; the API does not`](api-and-network.md#a-page-asks-before-it-deletes-a-file-the-api-does-not).
The two permanent refusals are one function, and it is the one the bank list reports with, so the
control a page draws and the answer the route gives cannot disagree.

**Both scanned folders, not just the one the machine writes to.** A file pushed to shared storage is
as much a mistake to undo as a downloaded one, and how a row arrived is not a distinction the person
deleting it has reason to care about.

**Behind the password, the same as choosing and fetching.** All three are one person deciding which banks
this machine keeps.

## Switching the bank while it plays

**`Ctrl+1`…`Ctrl+9` change the SoundFont without restarting the machine, keeping the song and its
position — and the whole feature is off until `debug.soundfonts` names a bank.** Comparing two banks
by ear otherwise means a restart each time, by which point the first bank is no longer in anybody's
ear, so the comparison the exercise is for is the one thing it cannot deliver.

**Slot 1 is the bank the machine resolved for itself, and is not configurable.** One digit that cannot
be misconfigured, needs no setting up, and is a fixed reference for the rest to be judged against.
Slots 2 to 9 come from `debug.soundfonts`, filled by `task soundfont:debug`.

**The level travels with the bank, and a bank without one puts yours back.** A slot carries the
`music_volume` its bank was measured to want, because an A/B where one bank is simply louder answers
the wrong question. The half that is easy to leave out is the other direction: a slot with *no*
measured level has to restore `audio.music_volume` rather than send nothing, because `Sticky` replays
the last volume it saw on every stream rebuild — so sending nothing attaches a leveled bank's
reduction to every unleveled bank pressed afterwards, **slot 1 included**, and the fixed reference
stops being fixed.

**`audio.soundfont` is never written back, and the *slot* is.** `debug.soundfont_slot` records the
slot, `Engine::start` opens it **instead of** the resolved bank rather than after it, and the measured
level travels with it exactly as it does for a keypress. Remembering nothing would start an evening of
comparing banks at slot 1 after every restart, and a restart is part of that exercise, because half of
what is being judged is how a bank sounds on the first song of the evening.

**The safety half is the important half.** A switch still does not write `audio.soundfont`, so
emptying `debug.soundfonts` returns the machine to the bank it resolves for itself whatever anybody
pressed — which is what makes the slots safe to leave configured. What is remembered is an *index
into a list that is itself the escape hatch*, so it cannot outlive the hatch being closed.

**It is a number and not a path**, which keeps it inside
[`Only `debug.` names a file`](foundations.md#only-debug-names-a-file) rather than pushing at it, and
makes every way of it going stale the same harmless answer: a shortened list, a cleared list and a
slot of 1 all mean the machine comes up on its own bank, silently. Slot 1 writes no entry at all, so
a machine that has never used the switcher has nothing in its settings file about it.

**Replacing the list clears the slot.** A slot number means whatever the list under it says, so
`--set-debug-soundfonts` clearing it is the only honest answer — the alternative is a remembered
number quietly naming a bank nobody chose.

**The name of the bank in force is on screen for as long as any slot is filled** — not a message that
appears on switching and fades. The reason to draw it is so a recording or photograph of a session
says which bank was heard, and a label that faded would be missing from exactly the frame somebody
kept. It sits under the key/tempo/melody badges.

**A video or MP3+G song gets the new bank without being interrupted, and the label says so.** Those
play through no synthesizer, so there is nothing to hear differently — and their audio is a decoder
feed moved into the audio thread that cannot be built again, so rebuilding the stream would end the
song rather than switch anything. The bank takes effect at the next MIDI song.

**Empty means off, and off means nothing is measured.** No keys, no label, and no bank but the
resolved one is ever opened — the rule this repository applies to `--frame-stats`, and why the
switcher can be compiled into every build without being a control panel in a product.

**There is an audible hole in the sound, and it is unavoidable.** `rustysynth`'s `Synthesizer` takes
its bank in `new` and offers no setter, and the synthesizer lives inside the audio callback, so a swap
means dropping the output stream and building another. What is *not* lost is the performance: the
machine keeps the parsed song, and a seek replays every channel's program and controller state onto
the new synthesizer, so the song comes back where it was and sounding like itself rather than like
sixteen default pianos.

**It is not on the API**, on the same judgment as `A partly loaded bank plays, and says so`: the
bank's HTTP surface is read-only, and a route for a debugging control would put it on a published
surface where seven places have to be kept in step. `GET /audio/soundfont` reports whichever bank is
playing, because that answer is not settled at startup.

## Nothing is left sounding

**The end of a song silences the voices, and a seek resets the channels.** Both were places where a
note could sound for the life of the process with nothing downstream to catch it:
`Sequencer::advance` early-returns for ever once finished, and a bare `Player` — `offline.rs`,
`render_wav` — has no watchdog behind it.

**The end of a song is an `all_notes_off`, not a `reset`.** The transport is deliberately left in
`Playing` so the reverb tail finishes, and only `reset` mutes that tail. `note_off_all` is also
*immediate* rather than a release, which is what cuts through a hold pedal left down — a CC123 would
be deferred by exactly the pedal it needs to escape.

**A seek is a `reset`.** `seek_ticks` replays controllers, program, bend and pressure from tick 0, so
resetting first loses nothing; what it gains is everything the replay cannot reach, because the replay
only sees what the file states *before* the target. A hold pedal pressed before the point being seeked
away from and never mentioned again stayed down, and `rustysynth` holds every released voice while it
is — so the next note-off on that channel deferred indefinitely.

**Measured, because "a pedal is sometimes left down" is a claim.** Of the parsed corpus files,
**31.00% touch CC64 at all**, 3.06% end with it down, and **2.55% hold it across a stretch long enough
for a seek to land inside** — one file in 39. Those three are deliberately separate: a pedal pressed
in the final bar strands nothing, and only the third is the defect. It is also the case where a stuck
note does **not** quietly decay, which is what makes a one-line change worth making.

**A pedal lift is an event, not a level.** The sequencer dispatches every event due in a block and
then renders it, and the synthesizer decides whether to release a deferred voice once per block — so
a file that writes a pedal-up and the pedal-down that follows it on one tick presents a pedal that is
down at both ends of the block, and the lift between them cannot be seen as a position. The fork
counts lifts on the channel and a voice records the count its note-off arrived at, so it releases on
the first block after a lift whatever the pedal is doing by then. Sequencers quantize a re-pedal onto
one tick, which is why this is a property of how files are written rather than a corner of timing.

**What it costs is not a note decaying unheard.** A whole part stays at full sustain and climbs, and
once the voice pool saturates it takes voices from channels that never touched the pedal. On the file
it was found on, a String Ensemble 2 part goes from 5 sounding voices to 39 and the piano beside it
from 6 to 165; rendered through FluidR3_GM the file runs up to 5.6 dB louder in ten-second windows
than the same file with the pedal honored, and falls back each time a lift does survive a block.

**Measured the same way, and it reaches the same order of file.** Of those, **2.56% cancel at
least one lift inside a block** — one in 39, and 8.27% of everything that touches CC64 — with 3.2%
of the corpus's lifts cancelled outright. Where it happens it is rarely occasional: the
worst file in the corpus defers 28,248 note-offs at once.

**The parser repairs most hanging notes before they reach here**
([`A half-read file plays, and says which track was lost`](songs.md#a-half-read-file-plays-and-says-which-track-was-lost)).
These two are the backstop for a song that arrived another way.

## The bank is chosen by id, not by path

**`audio.soundfont` names the bank id — the slug of a filename — and the SoundFont folder is what says
which banks exist.**

**A path is an instruction and an id is a name.** An instruction that cannot be carried out is worth
refusing over; a name that matches nothing is stale, not disobeyed, and is resolved against a folder
that is allowed to change. So a bank the folder no longer holds falls back to the bundled one rather
than to a sine test tone, which is a machine that sounds wrong for reasons nobody in the room can
discover.

**`chosen_by` carries `fallback`.** A machine on the bundled bank because nobody chose otherwise is
not the same machine as one on it *despite* a choice, and a client that collapsed them would hide
the only thing worth acting on. It is a `fallback` field of its own and deliberately **not**
`problem`, which means *why there is no bank* — the same line `A partly loaded bank plays, and says
so` draws for defects.

**`--set-soundfont` installs the bank before choosing it.** It still takes a path, because a path is
what somebody has; it hard-links or copies the bank into the machine's own SoundFont folder and writes
the resulting id. Without that step the flag would name a bank resolution cannot find, since
`task soundfont BANK=<name>` fetches into a cache shared between checkouts, which the machine does not
scan. **A different bank already there under the same name is refused, naming both** — deliberately
not given a `-2` the way a dropped package is, because a bank is hundreds of megabytes and two
confusable picker rows, and this route has a terminal to say so on.

**Nothing can name a file the machine does not own.** An installed-bank list that could include a path
from anywhere is the only route by which `Removing a bank` could delete a file outside the folders the
machine owns.

**The cost is on the older side.** A settings file written by this build, read by a binary that
took `audio.soundfont` as a path, reads the id as a path and gets a test tone; the remedy is
`--clear-soundfont`.

## A machine that is not making sound with a real bank says so on the screen

**The idle notice gains one line whenever the sound is not what it should be** — a stale bank setting,
a bank that would not parse, or no audio device at all.

All three otherwise reach only a log line and the API, and `A clash warns rather than only logging`
already decided what that is worth on this product: *"on an appliance under a television [a log line]
is indistinguishable from the package not being there."* The argument is stronger here. A missing
album is visibly missing; a machine whose instruments are all sine waves does not obviously look like
the *machine's* fault, and the person who could act on it is standing in front of it with no console
and no browser.

**It is a count and an area, not a sentence.** The line reads `1 problem: sound`, or
`4 problems: packages, sound` when both are true — see
[`A fault says how much is wrong, not what`](interface.md#a-fault-says-how-much-is-wrong-not-what).
The three states above are one number here; `/admin/`'s Problems tab is where they are told apart,
through the same `SoundFontStatus::complaint` this entry always delegated to.

**The naming order is packages before sound**, because a package problem means songs are **absent**
and names a file somebody can act on, where a sound fault names a setting on a machine that is still
playing. **The flash band wins the row outright**, for its own reason: two sentences in one place read
as one sentence.
