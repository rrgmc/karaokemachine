# Research: how far apart MIDI songs are, and whether a machine could level them

**Research only. Nothing implemented, nothing decided.** Levelling MIDI songs against each other
changes `Video and MP3+G play at the MIDI reference level` in
[`docs/decisions/audio.md`](../decisions/audio.md), which makes a MIDI song the untouched reference
the other two kinds come down to. That decision has to change before any of this is built.

Measured 2026-09-09 against the local corpus.

**Summary.** The song-to-song spread is **12.8 LU**, not the 7.4 the bank table's seven songs report,
and the songs that need answering are the quiet ones. A measurement rendered through the bank that
will play the song closes the spread entirely. A measurement rendered through some *other* bank closes
it to between 3.6 and 9.8 LU depending on which two, which across six banks is no better on average
than reading a level out of the events with no synthesizer at all. That estimate closes it to 5.0 LU,
costs 0.9 ms a song, needs no instrument bank, and clips **less** than the machine does today, because
the rule that raises a quiet song lowers a loud one. The staged packaging tools carry no bank, so the
estimate is the only one of the three that works unasked.

| Marker | Meaning |
|---|---|
| **[measured]** | Rendered and metered here. High confidence. |
| **[inferred]** | Reasoning over the measurements. A claim to test. |

## The harness, and what says it is measuring the right thing

`cargo run --release -p km-audio --example loudness_census` renders each file through one bank or
several, meters it with `km_loudness`, and prints the distributions and fits below. It renders the
way `tools/dev/soundfont-measure.sh` does: 44.1 kHz, music volume 1.0, the detected guide melody
channel muted, 90 seconds of each song. That matters because the figures it is compared against came
from that script.

**Over that script's seven songs it reproduces both banks to the decimal.** [measured]

| Bank | `soundfont-banks.conf` | The harness | Recorded spread | Range measured |
|---|---|---|---|---|
| colombogmgs2 | −21.0 LUFS | −21.05 | 7.4 LU | 7.4 |
| generaluser | −21.9 LUFS | −21.89 | 7.8 LU | 7.8 |

Muting the melody channel is what makes those agree. Rendering it audible puts both banks about 1 LU
loud, which is the difference between a mix and the same mix with a lead line over it.

The sample below is a fixed stride through the corpus's sorted `.mid`, `.midi` and `.kar` files, so
it spans every folder rather than the head of one, and a re-run reads the same files.
Byte-identical duplicates are dropped, because a corpus holding the same song twice would weight it
twice. Of 1,000 sampled: 3 duplicates, 3 that would not parse, 10 too quiet or too short for the
meter, **989 measured**.

## 1. The spread is 12.8 LU, and the seven songs said 7.4

[measured] 989 corpus songs, both banks, 90-second window.

| Bank | mean | sd | min | p10 | p50 | p90 | max | p90−p10 |
|---|---|---|---|---|---|---|---|---|
| colombogmgs2 | −22.80 | 5.11 | −48.5 | −29.5 | −22.1 | −16.7 | −11.3 | **12.8** |
| generaluser | −23.18 | 4.46 | −45.1 | −29.0 | −22.8 | −17.7 | −12.0 | **11.4** |

**Eight in ten songs fall inside a 12.8 LU band, and the whole sample spans 37 LU.** The complaint
this investigates is therefore larger than the one already answered: video and MP3+G were levelled
over a measured gap of 4.8 dB against a video and 16.4 dB against a commercially mastered MP3, and
this is 12.8 dB between one MIDI song and the next on the same machine at the same amplifier setting.

**The `spread` column ranks banks; it does not say what a room hears.** [inferred] Seven songs
chosen to compare banks give a number a hundredth the size of the corpus figure, and both are
correct for what they measure. A bank's own contribution to the spread is real and small beside the
files'.

## 2. A measurement carries between two similar banks, and not between any two

[measured] The same 989 songs on the bundled and the recommended bank, fitted against each other.

| | r | slope | residual sd |
|---|---|---|---|
| colombogmgs2 against generaluser | 0.954 | 0.833 | **1.33 LU** |

**The slope is 0.833 rather than 1.** A song 10 LU above Colombo's mean is 8.3 LU above the bundled
bank's, so a bank compresses the corpus as well as shifting it.

**That pair is the best in the matrix, and it is the pair a reader would take for typical.** [measured]
387 songs through six banks spanning the surveyed loudness range, every pair fitted:

| | generaluser | somsak | sc55 | aspirin | musescore |
|---|---|---|---|---|---|
| colombogmgs2 | **1.39** | 2.76 | 1.81 | 3.11 | 1.67 |
| generaluser | | 2.96 | 1.88 | 2.82 | 1.79 |
| somsak | | | 2.62 | 3.48 | 2.39 |
| sc55 | | | | 3.29 | 2.23 |
| aspirin | | | | | **3.81** |

Residual sd in LU. **The range is 1.39 to 3.81, and the median pair is about 2.6.** The synthesizer-free
estimate of section 3 sits at 1.86 to 3.30 against the same six, median about 2.55.

**So a measurement taken on an arbitrary bank is not better than taking no measurement and
estimating.** Rendering through Aspirin to level a Colombo machine leaves 3.11 LU where the estimate
leaves 2.27.

**Aspirin is the bank that breaks it, and its own numbers say why.** [inferred] It is the flattest bank
surveyed, 464 samples for 207 presets, and it measures the narrowest spread here at 10.7 LU against
Colombo's 13.7. A bank with few and uniform samples flattens the difference between instruments,
which flattens the difference between songs, so the corpus arrives compressed: the fit from Colombo
to Aspirin has a slope of 0.657. A bank that compresses cannot report how far apart two songs are.

## 3. The estimate reaches 2.3 LU, and its constants do not matter

The synthesizer-free estimate reads a level out of the events alone. Channel volume and expression
each scale amplitude by the square of their controller value, SF2's default velocity modulator is
close to the same shape, so one note contributes `(velocity/127)⁴ · (cc7/127)⁴ · (cc11/127)⁴` of
power, powers of simultaneous notes add, and the result is gated the way R128 gates a real signal.

[measured] Against the rendered figure over the same 989 songs:

| Bank | r | slope | residual sd |
|---|---|---|---|
| colombogmgs2 | 0.894 | 1.015 | **2.29 LU** |
| generaluser | 0.911 | 0.902 | **1.84 LU** |

**A slope of 1.015 says the physical model is right**: a decibel of estimate is a decibel of output.
What it cannot see is which instrument a program number selects, and that is where the residual
lives.

**Its three constants buy nothing, which is what makes 2.3 LU a property of the approach rather than
of a guess.** [measured] Residual sd against Colombo, sweeping each while holding the others:

| how long a held note counts for | 200 ms | 400 | 800 | 1500 | 3000 | no decay |
|---|---|---|---|---|---|---|
| | 2.48 | 2.41 | 2.34 | 2.29 | **2.26** | 2.30 |

| weight on the drum channel | 0.1 | 0.25 | 0.5 | 1.0 | 2.0 | 4.0 |
|---|---|---|---|---|---|---|
| | 2.54 | 2.41 | 2.32 | **2.26** | 2.32 | 2.54 |

| how long a drum hit counts for | 50 ms | 150 | 400 | 1000 | 3000 |
|---|---|---|---|---|---|
| | **2.25** | 2.26 | 2.28 | 2.30 | 2.30 |

Every setting tried lands between 2.25 and 2.54. The plateau is flat, and the defaults sit on it.

## 4. What each candidate would actually leave

[measured] The decisive table. Each column applies its own gain to all 989 songs and reports the
p90−p10 of what comes out, attenuating only. *Headroom* is how far below the bank's own mean the
target sits.

| headroom | untouched | measured on the bank playing | measured on the other bank | estimated, no bank |
|---|---|---|---|---|
| 0 LU | 12.8 | 6.7 | 6.7 | 8.6 |
| 2 LU | 12.8 | 4.7 | 5.2 | 7.1 |
| 4 LU | 12.8 | 2.7 | 4.2 | 6.3 |
| 6 LU | 12.8 | 0.7 | 3.7 | 5.5 |
| 8 LU | 12.8 | **0.0** | 3.6 | 5.2 |
| 10 LU | 12.8 | 0.0 | **3.5** | 5.1 |
| 12 LU | 12.8 | 0.0 | 3.5 | **5.0** |

Two of the three floor above zero, and each floors at its own error, which is about 2.56 times the
residual sd: 3.5 LU is the 1.33 LU Colombo-to-bundled residual seen as a p90−p10, and 5.0 LU is the
2.29 LU estimate residual seen the same way.

**The middle column is the best pair of six, not a typical one.** Read against section 2's matrix it
runs from 3.6 LU for that pair to **9.8 LU** for Aspirin against MuseScore, and the median pair gives
about 6.7. So the ordering that matters is:

| | residual spread left |
|---|---|
| nothing | 12.8 LU |
| rendered through some other bank, median pair | ~6.7 |
| estimated from the events, no bank | 5.0 |
| rendered through the bank playing it | 0.0 |

**Every one of them beats doing nothing, and only the last is clearly worth a bank.** [inferred] The
question a design has to answer is therefore not "render or estimate" but whether the bank doing the
measuring is the bank that will play the song. Where it is, rendering is unbeatable. Where it is not,
it costs three thousand times the estimate to land in the same place.

## 5. The quiet songs are the complaint, and only a boost answers them

**A song that is too quiet cannot be fixed by attenuating anything.** The complaint this began with
is files that sound right and play too low, and attenuation-only closes the spread by bringing
everything down to the quiet ones, which is the opposite.

[measured] Colombo, 989 songs, grouped by how far below the bank's mean they sit.

| how far below the mean | songs | boost to reach it | headroom under −1 dBTP | enough headroom |
|---|---|---|---|---|
| 0 to 3 LU | 212 | 1.4 dB | 7.4 dB | **100%** |
| 3 to 6 LU | 128 | 4.3 dB | 10.3 dB | 98% |
| 6 to 9 LU | 61 | 7.3 dB | 13.5 dB | **100%** |
| more than 9 LU | 56 | 12.1 dB | 18.7 dB | 98% |

**A quiet MIDI song has two to three times the headroom it needs.** [inferred] It is quiet because it
is sparse or lightly played rather than because it is compressed, so its peak falls with its loudness
and the room to raise it comes free. Nothing in the corpus behaves like a mastered recording, which is
what makes this different from the media case.

**And the boost costs nothing in level.** [measured] Bringing every song to Colombo's own mean:

| rule | spread left | mean level change | songs over −1 dBTP | worst peak |
|---|---|---|---|---|
| nothing done | 12.8 | +0.0 | 97 (10%) | +7.2 dBTP |
| rendered on the playing bank, bounded by its own peak | **0.0** | −0.0 | 1 (0%) | +0.0 dBTP |
| estimated, boosting blind, capped at +6 dB | 5.5 | −0.3 | 16 (2%) | +2.5 dBTP |
| estimated, boosting blind, capped at +12 dB | 5.0 | −0.0 | 18 (2%) | +2.5 dBTP |

**Boosting on an estimate clips less than doing nothing.** Ten percent of the corpus already exceeds
−1 dBTP on this bank at music volume 1.0, and the worst reaches +7.2. Levelling on the estimate leaves
2% marginally over, worst +2.5, because the same rule that raises the quiet songs lowers the loud
ones. The comparison that matters is against the machine as it stands, not against silence.

**Attenuating instead would cost 8.2 dB.** [measured] Reaching a 0.0 spread by attenuation alone needs
a target 8 LU below the bank's mean, and takes the whole catalog down with it.

## 6. The estimate cannot bound its own boost, and does not need to

The estimate's loudest single block is the only thing in it that could stand in for a true peak.
[measured] Fitted against the rendered peak over the rendered level, 989 songs: **r 0.454**, residual
sd **1.92 dB**. Compounded with the level's own 2.29 LU that puts a predicted peak at about 3 dB,
which is too loose to bound a gain against full scale.

**So a boost taken from an estimate is capped rather than bounded**, and section 5 says what that
costs: 2% of songs marginally over, against 10% today.

**A peak measured on one bank does not bound a boost on another either.** [measured] Colombo's true
peak against the bundled bank's, per song: **+0.81 dB mean, 1.85 dB sd**. Only a render through the
bank that will play the song licenses a properly bounded boost, which is the same conclusion section 2
reaches about the level.

## 6a. What the estimate's error is made of, and why none of it is worth correcting

The estimate's named weakness is that it cannot see which instrument a program number selects. That
is testable against the same 989 rows: take the residual against the rendered figure, and correlate it
with what each song is made of, as shares of the power the estimate itself counts.

[measured] The features that move it at all, by how much of the residual their own tenth-to-ninetieth
range accounts for:

| what a song is made of | mean share | r with the residual | LU across its range |
|---|---|---|---|
| piano | 0.224 | −0.294 | **−1.97** |
| reverb send (CC91) | 54.8 | +0.243 | **+1.17** |
| bass | 0.112 | +0.167 | +0.77 |
| ensemble | 0.059 | +0.161 | +0.44 |
| drum channel | 0.275 | −0.063 | −0.36 |
| **mean pitch** | 54.8 | **−0.006** | **−0.04** |
| **notes a second** | 19.1 | **−0.029** | **−0.16** |

**Two hypotheses die here.** Pitch was the leading one: R128 K-weights and this does not, so a
bass-heavy arrangement should read louder than it sounds. Power-weighted mean pitch correlates with
the residual at **−0.006**, which is nothing. A note's fundamental is not where its energy is, and a
bass patch carries harmonics through the whole band. Density is the other, on the reasoning that
summing powers understates a dense arrangement whose notes are not independent; it is also nothing.

**What survives is real and small.** A piano-heavy song renders quieter than the estimate says, which
fits: bank authors attenuate piano presets, and the estimate has no way to know. Reverb send is a
controller the estimate ignores outright and the synthesizer acts on.

**Corrections fitted on half the songs and scored on the other half.** [measured] The split is what
separates a real correction from eighteen numbers memorising 989 songs.

| correcting by | constants | fitted to | held back |
|---|---|---|---|
| nothing | 0 | | 2.27 LU |
| piano and reverb | 2 | 1.97 | 2.23 |
| piano, reverb, bass | 3 | 1.97 | 2.22 |
| every feature above | 18 | 1.76 | **2.06** |

**Two constants buy 0.04 LU on songs they have not seen, and eighteen buy 0.21.** The small models
look good only where they were fitted. The full one generalises and is still a ninth of the error.

**So the estimate is at its floor around 2.1 LU, and the floor is not made of anything nameable.**
[inferred] What is left is the difference between one preset and another inside a family, which is a
number in the bank rather than in the file. A correction cannot reach it without opening the bank, and
opening the bank is the render.

**The case that prompted this measurement is what settles it.** A real corpus file, *Pais Tropical*,
renders at −25.23 LUFS against the bank's own −22.80 mean, so it is 2.4 LU quiet and wants raising.
The estimate read it as 1.0 dB **loud** and the machine attenuated it: 3.4 LU in the wrong direction,
on a file somebody had already picked out by ear. The eighteen-constant correction would have left
3.1 LU of that. **A refinement that does not fix the case it was built for is not a refinement.**

## 7. What a measurement would cost to take

[measured] Per song, on this box, release build.

| | per song |
|---|---|
| Estimate, including reading and parsing the file | **0.9 ms** |
| Render 90 seconds through the bundled bank and meter it | 0.86 s |
| Render 90 seconds through Colombo and meter it | 1.13 s |
| Render the whole song through Colombo and meter it | 2.76 s |
| Measuring a media song, for comparison | ~0.6 s |

**Ninety seconds is not the song.** [measured] Over 149 songs measured both ways, the 90-second
window reads 0.40 LU quiet with a per-song sd of 1.12 LU, a worst case of 7.7 LU, and **12% of songs
differ by more than 1 LU**. A packaging measurement would have to render the whole thing, at 2.76 s a
song.

**The estimate is three thousand times cheaper and needs nothing installed.** `km-pack` already
parses every MIDI song and already runs `Analysis::of` on it, so the marginal cost is the estimate
itself.

## 8. The constraint that separates them

**A package is built on somebody else's machine, and the staged tools carry no instrument bank.**
`tools/dist/cmd.sh` stages each tool as its executable and a README, and says in place that there is
no asset-fetching step because none of them has a SoundFont. So rendering at packaging time needs an
answer to where the bank comes from: shipping 31 MiB beside `km-pack`, or asking the user to point at
an `.sf2` they happen to have, or fetching 262 MiB for the recommended one.

The estimate clears that outright. It also reaches the two cases a package cannot: a file played
straight from disk, and a package built before any of this.

**Letting a packager measure with whatever bank it can find does not clear it.** [inferred] Section 2
puts an arbitrary pair at 2.6 LU of residual against the estimate's 2.55, so a rule that searches for
a bank buys the accuracy of the free answer at the cost of a render, a bank, and `km-audio` in the
packager. What it does buy is the case where the searched bank is the playing bank, and that case is
worth having: it is the owner packaging for one machine they control.

**A stored figure has to say which bank produced it, and most banks cannot be interpreted.**
`soundfont-banks.conf` carries 63 bank rows and **15 `lufs` values**. A package measured with a bank
outside those 15 gives the machine a level it cannot convert to an offset, and the existing fallback
of −22.0 is 9.6 dB wrong for `somsak` at −12.4, which is worse than no levelling. Either measuring is
restricted to banks that carry a reference, or the packager stores the song's offset from the
package's own mean and the bank drops out of the record. [inferred] The second costs accuracy on a
small package: the standard error of a package mean is 5.11/√n, so 0.72 LU over 50 songs and 0.23 LU
over 500, and a package curated to one mood would carry a mean that is the mood rather than the bank.

**Measuring on the machine instead of in the packager is the one place the bank is certain, and the
appliance cannot afford it.** [inferred] The machine has the playing bank by definition, which is the
0.0 row. But the appliance renders at about 1.2× realtime under playback
(`docs/architecture/audio.md`), so an offline pass over a catalog there costs roughly the catalog's
own playing time.

## 8a. How far above MIDI real media actually sits

The decision this feature sits beside quotes one video at −17.1 LUFS and one MP3+G pair at −5.5. A
whole library measures tighter than either. [measured] `km-pack reanalyze` over 286 karaoke media
files, none of which carried a measurement before:

| kind | files | median | mean | range | at the −20 dB floor |
|---|---|---|---|---|---|
| MP3+G | 223 | −14.9 | −15.0 | −25.4 … −5.7 | **0** |
| video | 63 | −14.2 | −13.9 | −21.0 … −7.0 | **0** |

**The two kinds are the same loudness as each other**, within 1 dB of median, and both sit about
**8 dB** above the −22.8 a MIDI song is now brought to on the recommended bank. So the gap a room
hears between a MIDI song and a media one is that 8 dB, and it closes entirely once the media carries
numbers.

**Nothing reached `MIN_GAIN`.** The −20 dB floor exists to stop a wrong measurement muting a song,
and the hottest file in 286 is −5.7 LUFS, which asks for −15.3 dB. The floor has room to spare
against real material.

**The measurement is release-only in practice.** A debug build of `km-pack` decodes roughly thirty
times slower, which turns a two-minute job into seventy and prints nothing while it does it.

## 9. Two things the seven songs hid

**Colombo clips on real files.** [measured] Of 989 songs at music volume 1.0, **97 (10%) exceed
−1 dBTP** and the loudest reaches **+7.2 dBTP**. Its row calls it the only one of the leaders that
clips nothing, and it carries no `volume`, so `km-audio`'s `limit` is flattening one corpus song in
ten on a machine set that way. The bundled bank reaches +3.2 dBTP and is 0.81 dB quieter at the peak
on average.

`somsak` is worse and is not a leader: over 387 songs its mean true peak is **+1.15 dBTP**, its
loudest is **+25.1**, and eleven of those songs metered at or above full scale and were excluded from
that run as divergences rather than as loud songs. Its `volume` of 0.22 is what stands between it and
that, and the figure was derived from the same seven songs.

**The melody channel has to be muted to reproduce the bank table.** [measured] Audible, it adds 0.22
LU to the mean with a per-song sd of 0.59 LU and 10% of songs moving more than 1 LU. The machine's
own default is the guide melody on, so a figure taken with it muted is 0.22 LU below what the room
hears.

## 10. Not a cause: master volume SysEx

`km_song::Song` discards SysEx before a caller sees it, so General MIDI, Roland GS and Yamaha XG
master volume messages never reach the synthesizer. [measured] By byte scan over the same 989 files,
**none carries any of the three**. Honoring them would change nothing.

## What this leaves to decide

1. **Whether to level MIDI at all**, which is the change to
   `Video and MP3+G play at the MIDI reference level`.
2. **Whether the target is the bank's own mean.** If it is, all three kinds of song sit at the figure
   media is already brought to, and the media rule needs no change at all. Any target below that mean
   would make video the loud kind again.
3. **Whether a song may be raised.** `Player::set_song_gain` clamps at 1.0 and the architecture note
   calls that clamp load-bearing. Nothing else answers the complaint, and section 5 measures the cost
   at 2% of songs marginally over −1 dBTP against 10% today.
4. **Estimate, render, or both.** Rendering through the playing bank leaves nothing; rendering through
   an arbitrary one lands where the free estimate lands. A design that renders when the bank is named
   and estimates otherwise is what the numbers point at, and it stores two things in one field.
5. **How a stored level says which bank measured it**, given 15 references for 63 banks.
6. **Colombo clipping one corpus song in ten, and `somsak` diverging**, which stand on their own and
   are not about levelling.
