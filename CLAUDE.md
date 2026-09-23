# karaokemachine — repository guide

A karaoke machine like a commercial home unit. MIDI files, video files, MP3+G pairs, and UltraStar
and LRC files with their audio are the only song sources. A native cross-platform app, Windows, macOS and
Linux first and Android second, with synced word highlighting. An HTTP API covers search, queueing
and control.

**This file is conventions and pointers, and stays short**: it is loaded into context every session.
The detail lives in the documents below, opened when needed.

## The documents, and which answers what

| Read this | To answer |
|---|---|
| [`docs/decisions/`](docs/decisions/) | why something is the way it is, [indexed](docs/decisions/README.md) |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) + [`docs/architecture/`](docs/architecture/) | how it is built, and what was measured building it |
| [`BUILDING.md`](BUILDING.md) | how to compile, test and release it — **and the full command reference** |
| [`RELEASE.md`](RELEASE.md) | the order a release is cut in |
| [`CHANGELOG.md`](CHANGELOG.md) | what each release gave the person using it |
| [`DEPLOYING.md`](DEPLOYING.md) | how to get it onto a device — Android, iOS, and a Linux box over SSH |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | what to run before a pull request, and the invariants |
| [`docs/research/`](docs/research/) | investigations — findings, never commitments |
| [`docs/HISTORY.md`](docs/HISTORY.md) | why this took eighteen years and then fourteen days |
| [`README.md`](README.md) | for somebody who has the machine, not the source |

### Rules

1. **A change of technical direction updates the architecture notes in the same change as the
   code**, not afterwards. That includes a design that turned out wrong, a dependency swap, an
   assumption invalidated, a new risk found.
2. **A new product decision gets an entry in [`docs/decisions/`](docs/decisions/)**, in the file for
   its area. So does a changed requirement, because a requirement and the decision that serves it are
   the same entry. Do not record either only in an architecture note. That entry is the convention
   most worth protecting.
3. **A decision is authoritative over an architecture note.** Where the two disagree, the note is
   out of date — fix it.
4. **Research notes are not decisions.** Nothing in one is committed until it appears in
   `docs/decisions/`.
5. Do not create additional plan or status documents. This rule does not catch reference
   documentation. `README.md`, `BUILDING.md`, `CONTRIBUTING.md`, the per-port READMEs,
   `icon/README.md`, `docs/learning-rust.md` and `docs/HISTORY.md` say what *is* or *was* true, where
   a plan says what *will be*.
6. **Prose states the rule and the reason, and never how the rule was arrived at.** No "used to", no
   "no longer", no version or milestone numbers, no sentence whose subject is the document. No
   appositive tail restating the clause before it. **This governs code comments and everything the
   project publishes**, as much as documents: the release page, the changelog, the site, the words
   inside a program. Somebody writing up a change they have just made breaks it most easily. The full
   form, with the list of what creeps back, is
   [`How a document in this repository is written`](docs/decisions/repository.md#how-a-document-in-this-repository-is-written).
7. **A commit's subject states the rule that holds after the change, and its body states the fault
   the change answers.** The fault's past tense is the content, and the session's own is what fails.
   `task lint:prose` reads `origin/master..HEAD` and nothing below it, so `git commit --amend` fixes
   a message while the branch is unmerged.
8. **A sentence takes the active voice, one idea, and twenty-five words at most.** Simplified
   Technical English gives the rest: simple tenses, one word per idea, six sentences to a paragraph,
   no metaphor and no idiom. A path, a flag, a name and a number stay verbatim. **A full stop is not
   the only join**: two clauses carrying one idea keep their conjunction. A run of fragments is worse
   than the long sentence it replaced. **A heading keeps the voice it has**, and so does a table
   cell; `task lint:prose` counts the words and cannot see a metaphor.

## Non-goals

Standing decisions — do not add these without recording the change in `docs/decisions/` first:

- **No scoring of singers.** Nothing here rates a performance. The 0–10 number is a song *file*'s
  **suitability**, and it is called that everywhere — never `score`.
- **A song is a MIDI file, a video file, an MP3+G pair, or an UltraStar or LRC file with its MP3,
  and nothing else.** An UltraStar `.txt` names its MP3, and an `.lrc` shares its stem. No bare
  audio-file song sources: an MP3 or WAV on its own has no words in it. The machine reads an UltraStar file for its timed words only,
  and discards its pitches.
  **CD+G as a *disc* format is not supported** — `.bin`/`.cue` images and raw subcode rips — because
  the corpus holds none of them. The file pair is what the world actually trades.
- **No video backgrounds; wallpapers are still images.** A video *song* is not a video wallpaper.
- **No audio-file pitch shifting.** An MP3+G song answers a key change with a 409, exactly as a video
  does, because it has no key to change and never will.
- No in-app microphone DSP. Hardware mixes the microphones.
- **No shaped scripts — Thai, Arabic, Indic — and no right-to-left.** These need a shaper, and
  HarfBuzz is off by build decision. **CJK is not among them.** Han, Kana and Hangul need no
  reordering or joining, so they draw from a fallback face with the shaper still off. The television
  does draw them.
- **No removable-media modeling** — no mount detection, no eject, no "volume absent" state. A folder
  that is not there is a folder that is not there. A library may still live on an external drive
  named in `package_dirs`. What this refuses is the machine reasoning about whether a missing folder
  was removed or merely unplugged. **The catalog reconciliation leans on it**, so adding
  removable-media support reopens that rather than slotting in beside it.
- **Outside `debug.`, `settings.json` never persists the path of an individual content file.** A
  folder, yes. A file, no — and `debug.<kind>` is where naming one is legitimate, additively, one
  list per kind of file-based data.

## Nothing committed describes the machine it was written on

**No tracked file may name a local drive or folder, a home LAN address, personal hardware, or a
person.** Not in prose, not in a comment, not as test data, and not inside a published PNG. Real
paths, the corpus root, the appliance's addresses and this box's hardware belong in
`CLAUDE.local.md`, which git does not carry, and nowhere else.

- **A sample is invented, and a reproduction step names a variable.** `D:\tunes\karaoke` and
  `/tunes/karaoke` are the corpus samples the tests and docs use, `<your karaoke folder>` is what a
  reader substitutes, and `192.168.1.x` is the documentation address.
- **`tools/dev/check-no-local-refs.sh` will tell you**, and it runs first in `task check` because it
  is the cheapest failure to read. It matches by *shape*, so it does not itself contain anything
  private.
- **How large the corpus is stays out too.** A file count, a song count, a row count or a database
  size says how much music the owner holds. Write *large*, *a whole corpus*, *hundreds of thousands*,
  or the percentage the measurement turns on. A sample size stays — *4,000 files sampled from the
  local corpus* — because it names the work rather than the library.
- **It cannot see a brand name, and it cannot see a count.** A model number, a first-person aside, or
  a paragraph describing a room has no distinctive shape. One research note named the owner's own
  audio equipment fourteen times and passed every check for months. Six digits have no shape either.
  New prose needs a human pass.
- **It cannot see a built package either**, because it reads `git ls-files` and skips binaries. The
  same rule pointed outward is
  [`A package says nothing about the machine that built it`](docs/decisions/packaging.md#a-package-says-nothing-about-the-machine-that-built-it),
  and what enforces it is the package writer plus `km-pack check`.

The exemptions — the published identity, generic OS paths, second-person "on this machine" addressed
to the reader — are in the
[`What a committed file may say about the machine it was written on`](docs/decisions/repository.md#what-a-committed-file-may-say-about-the-machine-it-was-written-on)
decision. Read it before deciding something is an exception.

## Commands

**[`BUILDING.md`](BUILDING.md) is the command reference** — every command, with what each is for.
`task --list` prints what is routine. The four you will actually type:

```sh
task check          # local-refs, fmt, clippy, tests — the order a failure is cheapest to read
cargo km-build      # build the workspace, video included
cargo km-test       # the test suite, video included
cargo km-lint       # clippy over every target, -D warnings
```

### Traps — the short form

Each of these has cost real time. The reasoning is in `BUILDING.md`; these are the imperatives.

- **Never `--all-features` on this workspace.** It turns on `km-package-builder/desktop`, whose
  `wry` links libwebkit2gtk — which Linux deliberately never installs. Use `cargo km-test` and
  `cargo km-lint`, which name every feature explicitly.
- **The plain name is the video build.** `cargo km-build`/`km-test`/`km-lint` need ffmpeg's
  development libraries *and* libclang — `task ffmpeg` installs both, once per machine. The
  `-no-video` twins are what a fresh clone runs.
- **Do not type the feature list out.** It lives in `tools/setup/features.sh`; a third copy is the
  drift that arrangement exists to prevent.
- **Nothing may assume cargo builds into `target/`.** The directory moves. Ask cargo, through
  `dist_target_dir` in `tools/dist/common.sh`. Spelling `target/release/x` produces the worst failure
  this repository has had from one line. Cargo prints `Finished`, and the next line says the
  executable was not produced.
- **No test may bind a non-loopback address.** On Windows a test binary's path carries a build hash,
  so each rebuild raises a fresh firewall prompt and leaves a dead rule behind. Sixty of them
  accumulated once. `km-api`'s `on_ephemeral_port()` is the pattern.
- **Pair `--api-bind` with `--data-dir`**, or two machines on two ports still share one catalog, one
  packages folder and one settings file. That is not two machines. Prefer the full loopback form
  `--api-bind 127.0.0.1:<port>`: a listener on `0.0.0.0` raises a Windows firewall prompt per program
  and port.
- **On macOS prefix a bare `cargo` with `CMAKE_POLICY_VERSION_MINIMUM=3.5`** — CMake 4 rejects the
  vendored FreeType inside SDL3_ttf, and the panic names neither. **`task` sets it itself**, in a
  global `env:`, so `task check` and `task build` need nothing. The hand-typed `cargo km-build` is
  the only case left that does. It is the one point where the two spellings stop being
  interchangeable, and a deliberate exception argued in
  [`docs/architecture/distribution.md`](docs/architecture/distribution.md).
- **`master` takes pull requests only, and requires one check, `CI ok`.** A new CI job goes into
  `ci.yml` and under `ci-ok`'s `needs`. A workflow of its own with path filters would never report on
  the pull requests it skips, and those could never merge.
- **`gh pr create` takes a type label**: `--label bug`, `enhancement` or `documentation`. A workflow
  adds the program and platform labels from the changed paths, and cannot judge the type.
- **`gh pr merge` takes no `--delete-branch` here.** The repository deletes a head branch on merge
  itself, so the flag reaches only the *local* branch. Deleting that means `gh` checks out `master`,
  which the main checkout already holds. From a worktree, where the work is, that is
  `fatal: 'master' is already used by worktree` **after** the merge has gone through. The branch is
  merged, the remote copy is gone, and the message reads like a merge that failed. `git branch -d`
  from the main checkout is the second half, once the worktree holding the branch is removed.

## Working in parallel

**This repository asks for worktrees**, and `EnterWorktree` is deliberately gated on a repository
saying so. A session about to change tracked files **must** put itself in one first — call
`EnterWorktree`, or start with `claude -w` — rather than `git checkout -b` here.

**Every change, whatever its size.** Three or more agents are routinely at work here at once, and a
new one often starts while another is mid-edit. A session therefore cannot know it is the only one
running, and the answer can change while the work is in progress. Size is not what the rule is
about; the shared path is.

```sh
tools/dev/worktree.sh video-fix          # .claude/worktrees/video-fix, on branch video-fix
tools/dev/worktree.sh --remove video-fix # refuses if dirty; leaves the branch alone
```

**What must NOT be changed in a worktree** is a file git does not carry: **any `CLAUDE.local.md`,
wherever in the tree it sits**, and `.claude/settings.local.json`. The script copies every one of
them in, so a worktree holds its own *copy*. An edit to that copy is stranded where no merge will
reach it, and `--remove` will delete it. **Edit them in the main checkout.** Work touching both them
and tracked files is therefore two pieces of work with two homes, and splitting it is not optional.

The machine-local notes are split the same way the instructions are. `tools/platform/linux/` holds
this box's Docker and appliance notes, `crates/playback/km-video/` its ffmpeg paths, and the root
`CLAUDE.local.md` keeps a table saying which is which.

[`CONTRIBUTING.md`](CONTRIBUTING.md#branching-and-worktrees) holds the rest: why `target/` is
deliberately not shared, the Docker build-cache collision, and what a worktree carries that
`git worktree add` does not.

## Layout

The full tree, the dependency layering and what each crate owns are in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), and [`BUILDING.md`](BUILDING.md#layout) has the same
shape from the build's side. Six top-level directories. **`crates/`** is the library and the
binaries, **`tools/`** is everything you type or run, and **`ports/`** is the native application
shells. **`icon/`** and **`site/`** are the published identity, and **`docs/`** is the table above.

**Each of those carries its own `CLAUDE.md`**, holding the part that stops a mistake. The second
cargo workspace under `tools/cmd/assets/`, what `km-pack build` takes, why `fixtures/` is empty.
Nothing loads them now; they arrive when you open a file there.
