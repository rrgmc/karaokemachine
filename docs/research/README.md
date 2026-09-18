# Research notes

Investigations into things the project might or might not do — a file format, a library, an
approach. **They are findings, not commitments.** Nothing in one is decided until it appears in
[`docs/decisions/`](../decisions/).

Each states what was *verified* against what was *inferred*, so a later reader knows how much to
trust it. Add one when an investigation produces knowledge worth not repeating.

| Note | Subject | Bearing on the project |
|---|---|---|
| [`st3.md`](st3.md) | The Star 3 (`.st3`) karaoke format | Supporting it would be a **requirements change** — MIDI, video, MP3+G and UltraStar are the only permitted sources. Undecided. |
| [`ultrastar.md`](ultrastar.md) | UltraStar (`.txt`) syllable-timed lyrics beside an audio file | Read for the synced lyrics only. It reuses the MP3+G audio path and the MIDI lyric renderer. A local collection measures the dialects: unversioned files, decimal commas, legacy code pages and 18% relative mode, which converts to absolute beats. Public content is text without audio. **Decided**: see `UltraStar as a song source` in `song-sources.md`. |
| [`kar-formats.md`](kar-formats.md) | What a `.kar` file puts in its lyric stream | **Findings only.** Five formats share the extension, and the lyric stream also carries credits, legal notices and section labels — three ways, one of which announces itself. |
| [`midi-loudness.md`](midi-loudness.md) | How far apart MIDI songs are, and what could level them | Levelling them is a **change to a decision** — `Video and MP3+G play at the MIDI reference level` makes MIDI the untouched reference. Undecided. |
| [`webos.md`](webos.md) | The machine on an LG television | A platform is a **product decision** — it would need a row in `docs/decisions/distribution.md` beside `What the machine *is*, on iOS`. Possible, gated on there being no SDL3 for webOS. Not being built. |
| [`tizen.md`](tizen.md) | The machine on a Samsung television | A platform is a **product decision**, and this one is a second implementation rather than a port: no native code, and no listening socket for the API. A Google TV device on HDMI answers it today. Not being built. |
| [`linux-desktop.md`](linux-desktop.md) | The machine and the tools in an X11 or Wayland session | **Findings only.** SDL prefers XWayland unless the compositor advertises fifo-v1, so the packaging matches the backend a desktop session actually takes — and no check here has ever opened a window on Linux. |
| [`sqlite-mmap.md`](sqlite-mmap.md) | What a page of the curation tool costs, and how its pages reach the program | **Findings.** Anything a covering index answers is milliseconds even cold; anything that visits rows at random is seconds, and that is the whole of what a cold browse waits on. The mapping helps warm and is unresolved cold. One index came out of it, `songs_countable`; two seek-per-row queries are named as worth a measurement of their own. Also: how a verified cache purge still produced warm figures. |
