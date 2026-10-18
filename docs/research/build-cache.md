# A compilation cache shared between worktrees

What [mr-boxington](https://mr-boxington.jdx.dev/) (`mbx` 1.24.1) saves on this workspace, and
whether it can hand one checkout another checkout's output. Measured on the Windows development box,
2026-10-18, with toolchain 1.98.1 and no other build running. Every figure is one run.

**Why it was looked at.** A worktree builds into its own `target/`, so each new one compiles every
dependency from cold. A shared `CARGO_TARGET_DIR` was tried and removed. See "`CARGO_TARGET_DIR` is
not set, and should not be" in [`CONTRIBUTING.md`](../../CONTRIBUTING.md). mbx keeps each `target/`
and caches individual compilations in a store keyed by their inputs.

## How it was set up

- `cache_dir` on the same NVMe drive as the checkout, not the default under the user profile.
- `target.views = false`, so `target/` stays in the checkout. mbx's default moves it behind a
  symlink into its store, which is the arrangement this repository removed.
- Cross-checkout target seeding needs Cargo 1.100. It played no part.
- Three fresh worktrees from `tools/dev/worktree.sh --no-assets`: one for plain cargo, one that
  filled an empty store, one that read the store the second had filled.

## What it saves (verified)

Wall time in a fresh worktree, each row run after the one above it in the same worktree:

| Command | plain cargo | mbx, empty store | mbx, filled store |
|---|---|---|---|
| `cargo km-build` | 138 s | 132 s | **81 s** |
| `cargo km-test --no-run` | 61 s | 87 s | 73 s |
| build and test together | 199 s | 219 s | **154 s** |
| `cargo build --release -p karaokemachine` | 175 s | 189 s | 161 s |

- **A new worktree's first build and test costs 23% less.** The build alone costs 41% less.
- **The test build costs more than plain cargo**, by 12 s with a filled store. Most test binaries
  are links mbx cannot cache, and it pays its own bookkeeping on each.
- **A release build saves 8%.** Thin LTO with one codegen unit dominates, and mbx cannot cache it.
- **The first build into an empty store costs up to 10% more.**
- **Rebuilding a checkout the store has already seen is 23 s.** Two worktrees with `target/`
  deleted, built at the same time: 517 hits and no misses each, no `Blocking waiting for file lock`,
  25 s for both.

Hits with a filled store: 477 hits and 36 misses for the build, 62 and 45 for the test build, 373
and 25 for the release build.

## What it does not cache (verified, from `MBX_BYPASS_LOG`)

- **A workspace member's first build in a new checkout.** `share_workspace_root` is off by default,
  so a member's key includes its checkout. That is the safety property below.
- **Every link that carries a Windows resource file.** `karaokemachine`, `km-package-builder`,
  `km-package-simple`, `km-remote` and `km-song-sync` link a `resource.res` from their build
  scripts, and mbx reports it as an `unmodeled-link-argument`. That covers their test binaries too.
- **`cdylib` and `staticlib` crates**: the Android and iOS libraries.
- **Five build scripts on a first build**, reported as `failed to read Cargo's build-script
  declarations`.

## Whether a checkout can receive another's output (verified)

**It did not, in the case that removed the shared target.** A test asserting
`env!("CARGO_MANIFEST_DIR")` equal to the runtime `CARGO_MANIFEST_DIR` was put, byte-identical, in two
worktrees. The second run took 47 units from the store and compiled the test crate itself. Each
reported its own directory.

**`MBX_VERIFY=1` reports 79 of 85 restored units as different bytes.** These are `.rlib` and
build-script `.exe` files, plus rustc's artifact notices. Plain cargo does the same: two builds of
`smallvec` and `windows-link` into two target directories produced different `.rlib` files, each
naming its own directory. The divergence is the target path. mbx documents restored output as
"equivalent, not always identical".

## Costs (verified)

- **The store held 14.8 GiB** after debug, test and release builds in two worktrees. Its default
  budget is 5% of the cache disk, and pruning runs by itself.
- **No crash** across the runs, on a box where process creation during cargo builds has triggered a
  third-party driver's fault.
- `mbx analyze` resolved a worktree to the main checkout and found no recorded build. Not pursued.

## Inferred, not measured

- The 36 misses of a filled-store build are close to the workspace's member count, so the native
  dependencies (SDL3, SDL3_ttf, the bundled SQLite) most likely hit. The misses were not listed by
  name.

- The saving falls as a branch drifts from `master`: a changed `Cargo.lock` misses for every
  dependency it changes.
- A toolchain bump invalidates every rustc entry at once. The FAQ says so.
