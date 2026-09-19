# `km-pack`

Two traps, each of which has cost real time.

- **`km-pack build` takes a description, never a folder.** `km-pack spec` walks a folder and writes
  down every decision it would otherwise make silently; `build` does what the file says.
- **A build refuses a song with no language.** Set it in the builder, name it in the `--index` CSV,
  or pass `--default-language`. `und` is the standard's own "undetermined", and it is the honest
  answer.

**Use `--release` for a scan over a real corpus.** A debug `analyze` over a full cache took 78
minutes against single-digit ones optimized.
