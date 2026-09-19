# Test material

**This repository commits no song.** `km_song::testing` writes every karaoke file the tests use, and
it produces standard MIDI bytes by hand. The parser therefore meets bytes we control end to end, and
each fixture pins exactly the shape it is named for. See the
`Every fixture in the tree is synthetic` decision in `docs/decisions/`.

Nothing under this directory is tracked. Both folders below are gitignored, and neither one builds or
tests anything: the test suite compiles its fixtures in rather than reading them from disk.

| Folder | What it is |
|---|---|
| `generated/` | The synthetic fixtures written out as files, for looking at. Rebuild at any time — nothing depends on them existing. |
| `scratch/` | Yours. Ad-hoc experiments over a larger sample from a corpus that lives outside the repository. |

Writing the fixtures out:

```sh
cargo run -p km-song --features testing --example write_fixtures      # -> fixtures/generated/
cargo run -p karaokemachine -- --play fixtures/generated/soft_karaoke_header_on_words_track.kar
cargo run -p km-lyrics -- dump fixtures/generated/soft_karaoke_header_on_words_track.kar
```

It writes two sets. `km_song::testing::FIXTURES` are files that must **play**, one per shape the
parser has to handle. `UNREADABLE_FIXTURES` are files it must **refuse**, which is the other half of
what a parser answers for. Pointing a tool at one of those is how you see what it says
about a bad file.

**When a real file exposes a bug, distil it.** The shape goes into `crates/song/km-song/src/testing.rs`
as a minimal synthetic fixture named for the case it covers, and the file itself stays wherever you
found it. That is the rule this repository runs on, and `crates/song/km-song/tests/synthetic_corpus.rs`
sweeps whatever the module produces, so adding one is a single edit.
