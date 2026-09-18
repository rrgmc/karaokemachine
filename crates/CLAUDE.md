# `crates/`

**Five folders, and they are the dependency layering written down**: `song/` is what a song is and
the catalog of them, `playback/` turns one into sound and picture, `machine/` is the machine itself,
`remote/` is the singer's remote in its pages and its five hosts, and `platform/` is what a host asks
for rather than what karaoke does — the operating system, and the person in front of it.

**They are organizational and enforce nothing** — `remote/km-remote-pages` taking `machine/km-api` is
correct. The full tree and what each crate owns are in
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).
