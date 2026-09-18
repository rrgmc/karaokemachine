# `fixtures/`

**This repository commits no song.** Write the fixtures out before anything that plays one:

```sh
cargo run -p km-song --features testing --example write_fixtures
```

**When a real corpus file exposes a bug, distil it** — a minimal synthetic fixture in
`crates/song/km-song/src/testing.rs`, named for the case it covers. The file itself stays where you
found it.

The two fixture sets, and what is gitignored here, are in [`README.md`](README.md).
