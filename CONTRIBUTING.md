# Contributing

How work gets done in this repository. [`BUILDING.md`](BUILDING.md) is the prerequisite —
this file assumes you can already build and test.

## Before a pull request

```sh
task check          # local-refs, prose, the two pins, fmt, clippy, tests — cheapest failure first
```

`task` is optional and forwards to cargo aliases; each of these works with it absent:

```sh
tools/dev/check-no-local-refs.sh     # or: task lint:local  — runs first, cheapest failure to read
tools/dev/check-prose.sh --changed   # or: task lint:prose  — the lines this branch adds
tools/dev/check-prose.sh --commits   # ...and the messages it adds them in
tools/dev/check-toolchain-pin.sh     # or: task lint:pin
tools/dev/check-version-pin.sh       # or: task lint:version
tools/dev/check-cargo-config.sh      # or: task lint:cargo  — every value a worktree can inherit
tools/dev/labels.sh check            # or: task lint:labels — a label for every platform and program
cargo fmt --all
cargo km-lint                        # clippy over every target, -D warnings
cargo km-test                        # the test suite
cargo km-lint-assets                 # and the same two over tools/cmd/assets, which is a second
cargo km-test-assets                 # workspace `--workspace` cannot reach
```

**The compiler version is not yours to choose.** `rust-toolchain.toml` pins an exact one, and rustup
installs it on the first `cargo` command. There is nothing to select, and `rustup update` is not part
of building this. Bumping it is a deliberate commit, and
[Bumping the Rust toolchain](BUILDING.md#bumping-the-rust-toolchain) has the procedure and the two
things it silently breaks.

**Never use `--all-features` on this workspace.** It is not a style rule. The flag turns on
`km-package-builder/desktop`, whose `wry` links libwebkit2gtk. `tools/platform/linux/apt-deps.sh`
deliberately never installs that library, because a Linux build of that tool has no window by design.

On Linux the one flag meaning "everything" therefore asks for the one thing the platform is designed
not to provide. It works on Windows and macOS, where WebView2 and WKWebView need nothing at build
time. The machine it was typed on could not see the fault.

The feature list therefore lives once, in `tools/setup/features.sh`, and
`tools/platform/linux/check.sh` asserts that the copy in `.cargo/config.toml` agrees with it. Adding
a feature to a crate means adding it there.

On Linux, or before a push from any platform:

```sh
tools/platform/linux/check.sh        # fmt + clippy + tests on Linux, in Docker; or: task check:linux
```

Worth running. Three failures appear only off Windows: `Path` treating `\` as an ordinary character,
a missing system library, and a wrong `#[cfg]`. All three have happened here.

## Branching, and worktrees

**Never work directly on the default branch.** Branch before the first edit, not before the commit,
so there is never a working tree full of changes sitting on `master`.

**This repository asks for worktrees**, and the reason is concrete. Several agents and sessions are
routinely at work here at once, and a session cannot know it is the only one running. Two sessions
editing one checkout clobber each other, and small work is exactly the work somebody does in place.

```sh
tools/dev/worktree.sh video-fix              # .claude/worktrees/video-fix, on branch video-fix
tools/dev/worktree.sh --list
tools/dev/worktree.sh --remove video-fix     # refuses if dirty; leaves the branch alone
```

`git worktree add` does nine tenths of that. The tenth is why the script exists. Several things this
repository needs are deliberately not committed, so a plain worktree is a checkout that half works.
See [`docs/architecture/`](docs/architecture/) and the script's own header for what it carries
across.

**A worktree lives at `.claude/worktrees/<name>`, inside the checkout.** A worktree there is inside
the permission root a Claude Code session already holds, and one beside the repository is not.
`tools/dev/worktree.sh --where <name>` prints the path, and it is the only place the rule is spelled.
See [`A worktree lives inside the checkout`](docs/decisions/repository.md#a-worktree-lives-inside-the-checkout)
for the two things the location costs, and what answers each.

Four things that cost time when learned the hard way:

- **A worktree never inherits uncommitted changes.** Commit or stash first.
- **Every value in `.cargo/config.toml` is a string, and an array breaks every worktree at once.**
  Cargo reads that file from the checkout as well as the worktree, and joins arrays where it
  replaces strings. An array alias therefore works where it is written, and expands to
  `build … build …` everywhere else. `task lint:cargo` refuses one. A command that needs an argument
  with a space in it goes in `Taskfile.yml`, which Task reads from one directory and never merges.
- **`target/` is not shared, on purpose**, and nothing sets `CARGO_TARGET_DIR`. Each worktree builds
  into its own `./target`. See "`CARGO_TARGET_DIR` is not set, and should not be" below for the
  afternoon that established this.
- **Running the machine in two worktrees collides outside git entirely.** The data directory comes
  from the platform's config folder, not the checkout, so every worktree shares one `settings.json`,
  one catalog and one packages folder. Give each a `--data-dir` and its own `--api-bind`.

## What a commit message says

**The prose rule reaches a commit message**, and the standing decision is
[`How a document in this repository is written`](docs/decisions/repository.md#how-a-document-in-this-repository-is-written).
The subject states the rule that holds after the change, in the declarative `git log` is written in.
The body states the fault the change answers and why this is the answer to it.

**The past tense a commit is allowed is the fault's, not the session's.** Naming what was broken is
the content, the way a `Fixed` entry in the changelog is. What fails is the route somebody took to
the fix. The attempts, the order things were found in, a count before against a count after, a commit
or a message referred to as a thing.

```
fix(dist): the table's commands are ones that can be typed
feat(builder): a filter lives as long as the folder it names
docs(decisions): a release keeps what it changed
```

**The sentence shape reaches a message too.** The active voice, one idea to a sentence, twenty-five
words at most, and the same decision sets the rest of it out. A subject is a sentence like any other.
A long body becomes more short sentences, never fewer facts, and two clauses that carry one idea keep
their conjunction rather than becoming two fragments.

**A document you convert to that shape joins `tools/dev/prose-converted.txt`.** The checker then
reads it whole in every mode, so nothing slides back. Run `tools/dev/check-prose.sh` over the tree
before you add the line.

`task lint:prose` reads the subject and body of every commit above `origin/master`. A message is
therefore judged while the branch is unmerged, where `git commit --amend` or `git rebase -i --reword`
still costs nothing. **A message already on the default branch is out of its reach.** Rewording one
changes every hash below it, which costs a reader more than the sentence saves.

## Where a decision gets written down

Two documents carry what the code cannot say for itself, and **both are authoritative**:

| File | Role | When it changes |
|---|---|---|
| [`docs/decisions/`](docs/decisions/) | one entry per product decision, with its reasoning | when a decision is made or reversed |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) + [`docs/architecture/`](docs/architecture/) | how it is built, and what was measured building it | continuously, with the code |

**A change of technical direction updates the architecture notes in the same change as the code**,
not afterwards. A new product decision gets an entry in `docs/decisions/`.

**Research notes are not decisions.** `docs/research/*.md` record investigations into things the
project might or might not do. Nothing in one is committed until it appears in `docs/decisions/`.

## Nothing committed describes the machine it was written on

**No tracked file may name a local drive or folder, a home LAN address, personal hardware, or a
person.** Not in prose, not in a comment, not as test data, and not inside a published PNG.

`D:\tunes\karaoke` and `/tunes/karaoke` are the invented corpus samples the tests and docs use,
`<your karaoke folder>` is what a reader substitutes, and `192.168.1.x` is the documentation
address. `tools/dev/check-no-local-refs.sh` matches by *shape*, so it contains nothing private
itself, and it runs first in `task check`.

**A clean run does not mean the tree is clean.** The script checks three shapes: a drive-letter path,
a private-range address, and a named home directory. The rule above forbids two more things that have
no shape at all. A hostname, a model number, an email address and a person's name read as ordinary
prose. So **new prose needs a human pass**, and it is not optional:

- The research note that named the owner's own audio equipment fourteen times passed every check for
  months. Reading it is what found them.
- `km-package-builder`'s `is_loopback` test used the appliance's real hostname as its not-loopback
  case. That is the "not as test data" clause, in the one place nobody reads for prose.
- A decision and a `km-song` test literal quoted a stranger's email address and telephone number out
  of a corpus file. Somebody else's person is still a person.

Four things are deliberately exempt: the published identity, generic OS paths, second-person "on this
machine" addressed to the reader, and stock hardware class names.
[`What a committed file may say about the machine it was written on`](docs/decisions/repository.md#what-a-committed-file-may-say-about-the-machine-it-was-written-on)
argues them. Read it before deciding something is an exception.

---

The rest of this file is what was learned building the thing — kept because each item cost real time
to find.

## What CI runs

**`master` takes changes through pull requests only, and the one check it requires is `CI ok`.**
Everything in `ci.yml` starts on every pull request, every push to `master`, a Sunday schedule and on
request. `CI ok` fails when any job failed, and passes when the rest passed or skipped.

| Job | Runs on | Checks | Skips when |
|---|---|---|---|
| `guards` | ubuntu | every text check `task check` runs, with prose over the whole tree, commit messages on a pull request, shellcheck, fmt for both workspaces | never |
| `linux` | `debian:13-slim` | `cargo km-lint`, `cargo km-test`: the video build | only Markdown or `docs/` changed |
| `desktop` | windows, macos | `cargo km-lint-no-video`, `cargo km-test-no-video` | only Markdown or `docs/` changed |
| `assets` | ubuntu | `cargo km-lint-assets`, `cargo km-test-assets` | that directory, the toolchain pin and `ci.yml` are untouched |

**Why one workflow with skipping jobs, and not a workflow per concern with path filters.** GitHub
puts path filters on the trigger. A required check from a workflow that never started never reports,
and a pull request waiting on it never merges. A job whose `if` is false reports as skipped, which
branch protection accepts.

**The code filter is written as exclusions.** A new directory therefore counts as code until somebody
lists it, and a move makes a job run rather than skip. The `assets` filter is an inclusion and can
fail open after a move. The weekly run is its backstop.

**Linux runs in `debian:13-slim`** because that is the ffmpeg the appliance and the `.deb` link, and
Ubuntu's is an older major. Windows and macOS take the `-no-video` build. They find ffmpeg through a
download `tools/setup/fetch-ffmpeg.sh` owns, and the code behind the `video` feature has no
platform-specific paths.

**The aliases are the feature lists.** Every job runs a `cargo km-*` alias, so `ci.yml` holds no copy
of a feature string, and `check.sh` asserts the aliases against `tools/setup/features.sh`.

**No separate `cargo build` step**: `clippy --all-targets` type-checks every target and `cargo test`
compiles and links them. **Caches are saved from `master` only**, so pull requests read one cache per
job rather than each writing their own into the repository's 10 GB.

- **`pages.yml`** publishes the landing page when `site/`, its pictures, its favicon or
  `tools/dist/site.sh` change. It has no `pull_request` trigger, because a pull request has no
  deployment environment, and it is never a required check. It queues rather than cancels, because a
  cancelled deploy can leave the site part-published.
- **`release.yml`** builds the release carriers from a pushed `v*` tag and fills the draft release.
  It has no `pull_request` trigger and is never a required check. See
  [`Releases`](BUILDING.md#releases) for the secrets it needs.
- **`dependabot.yml`** opens one grouped pull request a month for the actions. Cargo is left out: a
  dependency bump wants the Android and iOS builds, which run from a tag and never on a pull
  request.

**`tools/platform/linux/check.sh` is the local twin of the `linux` job.** The same fmt, clippy and
tests, in the image that builds the `.deb`, and about a minute against a warm cache.

## The shared build cache, and what it costs

`check.sh` and `deb.sh` share a named Docker volume deliberately. A cold Linux build of SDL3 and the
bundled SQLite is minutes. The whole value of a local check is that it finishes while you are still
thinking about the change. But **both mount the working tree at `/src`, and a git worktree
mounts at `/src` too.** Cargo's fingerprints record that path, so the volume cannot tell one checkout
from another. It will hand a run artifacts compiled from different source at the same location.

**What that looks like is the reason it is worth a paragraph.** It is not a cache-shaped failure but
a *compiler* one, naming code you are looking at and can see is right. A missing field on a struct
whose definition has it, or a wrong arity on a function that plainly takes one argument. The second
symptom cost more than the first. It also reported an error in a crate the branch had not touched,
**which reads like somebody else broke `master`**.

```sh
# cygpath is the Git Bash spelling, and MSYS2_ARG_CONV_EXCL stops MSYS rewriting /src on the way in.
MSYS2_ARG_CONV_EXCL='*' docker run --rm \
  -v "$(cygpath -m "$PWD")":/src -v karaokemachine-deb-build:/build \
  "$(. tools/platform/linux/image-tag.sh; echo "$IMAGE")" \
  bash -c 'cd /src && cargo clean -p km-audio -p karaokemachine'
```

**For a check, the fix is knowing the shape.** Giving each worktree its own volume trades a rare
confusion for a guaranteed multi-minute rebuild in every one.

### …except where the artifact ships

**That conclusion holds for `check.sh` and nowhere else, and the line between them is what the
artifact is for.** A check that goes wrong hands you a baffling compiler error and costs ten minutes.
A *release* build that goes wrong hands you a package compiled from another checkout's source. **No
error at all, and nothing downstream that would catch it.** In the `.deb`'s case the deploy script
then installs it on the appliance.

So both release paths clean their own crates first, workspace members only, with **the crate list
derived from the manifests rather than written out**. One crate's package name differs from its
directory, so a hand-kept list has to remember that. One that forgets **protects everything except
the thing being shipped.**

**That clean is conditional, and the invariant is one sentence:**

> **The stamp names whoever last wrote to the shared target directory. Every writer records — unless
> recording would erase a mismatch it did not resolve. Only the release paths react to a mismatch.**

The first clause is why `check.sh` records without cleaning. A writer that did not record would make
the release paths' skip unsafe. Build from A, check from B, build from A again: A's stamp still
matches, so the clean is skipped over B's artifacts.

**The middle clause covers a case the first one walks into.** Recording does two jobs where only one
is wanted. It makes a *later* release build clean, and it also **erases a mismatch that is still
outstanding**, because recording alone cleans nothing. The mirror image of the sequence above was the
live state of the box after a worktree was removed. Recording therefore leaves a stamp alone when it
names somebody else. The *direction* is what makes that safe to reason about: **preserving a mismatch
can only ever cause more cleaning, never less.**

It also explains why the shorter rule looked complete and was not. "Every writer records" comes from
a single scenario in which the *other* checkout wrote last. **The rule it produces is correct there
and wrong in the reverse**, and nothing distinguishes the two without asking which profile was
written.

**The strictly more correct design, deliberately not taken.** A target directory per checkout would
remove the clean, the stamp, the race and the footgun at once. It costs a full cold build per
checkout, and several GB apiece in a VHDX that never shrinks. **At two checkouts that arithmetic does
not work. Revisit it at three.**

**Two more collisions, both settled by letting the name carry the identity** rather than by sequencing
anything:

- **The image tag.** All three drivers built the same tag, so a checkout whose branch touched the
  `Dockerfile` silently re-tagged the image another was about to run. It is now hashed from the
  `Dockerfile` and the dependency script: **equal inputs share one image, different inputs cannot
  overwrite each other.**
- **The tarball's ffmpeg prefix.** One fixed path plus a stamp, where a run whose stamp disagreed did
  `rm -rf` and rebuilt. With two checkouts that deletes a tree a peer may be compiling or running
  against, with no lock. Each keeps judging the other's stale, so **they ping-pong through a
  multi-minute rebuild every time either runs.** The prefix is now hashed from the release and the
  configure flags, and the install goes through a staging directory and a rename. A tree therefore
  appears whole or not at all, and **nothing deletes anything**.

**Nothing prunes either of them automatically**, and that is the same reasoning once more. Pruning is
precisely the act that would delete the other session's.

**What this does not fix, so a slow run is not misread as a broken one.** Two containers doing heavy
bind-mount I/O through WSL starve each other badly: a `dpkg` unpack that takes seconds alone took
nine minutes beside a peer. That is host contention, not a naming collision. **Suspect it first when
`check.sh` fails on something the Windows build compiled happily.** That combination is nearly always
this, because the faults `check.sh` exists to catch do not usually present as a type error about your
own struct.

## The Taskfile

`Taskfile.yml` is the one listing of what this repository does. **It wraps.** Every task is a line or
two of something else, a cargo alias or a script under `tools/`. That something else stays
authoritative, because it needs nothing installed. **`task` is therefore optional**: no build, no
test, no CI step and no release calls it.

**No task may carry a flag, a default, a feature list or an ordering the command it wraps does not.**
`task test` *is* `cargo km-test` and would be a bug if it were anything else.

**The version comes from `cargo pkgid`, not from the version helper.** That helper reads a version by
running the freshly built binary, and it is right to. Every shipped crate inherits its version, so
reading a manifest means picking the right one of many lines. **The run tasks cannot use it, because
they need the version *to find* the binary.** `cargo pkgid` needs no build and cannot disagree, since
a clap version is the cargo one.

Three things this cost, all Windows, all found by getting them wrong:

- **`bash` on the Windows PATH is WSL's**, a different git looking at a different filesystem with no
  MSVC cargo behind it. The `SH` variable derives Git Bash from `git --exec-path` instead. **The
  consequence deserves stating in its own right.** With that, and with Task running every command
  through its own embedded shell, `task` works unchanged from `cmd` and PowerShell. Nothing has to
  run from a Git Bash.
- **Task's shell is POSIX but its PATH is the host's.** There is no `sed`, `grep`, `awk` or `jq` on
  that box, so `cargo`, `rustc`, `git` and shell builtins compute every variable.
- **A glob expands with the host's separator**, so a `${e##*/}` strips nothing. The first `clean:old`
  **matched nothing while reporting success, which is the worst failure a cleaner can have.** That
  work lives in a script now, because Task's shell has no `rm` either.

**`clean:old` has no special cases at all, and that is the property to keep.** It removes an entry
only when its name begins with its app's own name, followed by a version that is not the wanted one.
Anything unrecognized survives, and the macOS bundle carries no version at all and never matches. It
knows one version, because there is one. `km-admin` and `km-wallpaper-pack` are in the excluded
workspace and follow the machine. Each needing an arm of its own, here and in `tools/dist/bin.sh`, is
what broke `task dist:bin`.

**The coupling to watch.** The Taskfile and the clean script both reconstruct the release layout,
which is `dist_dir()`'s rule to own. **The Taskfile has to keep its own**, because Task's embedded
shell cannot source a bash file. So does the clean script: every helper in the shared library
*builds* the layout, and that script is the one that takes it apart.

## Invariants worth knowing before you break one

### A change to what the analysis decides bumps `ANALYSIS_REVISION`

A curated corpus keeps the conclusions and not the evidence. A build that decides something new about
a file therefore cannot correct what is already stored. `km_suitability::ANALYSIS_REVISION` sits beside
every song, and it is how a scan knows to read that song again. Leave it alone after changing what a
scan would write, and the corpus keeps reporting what the build before you decided. It does that
silently, because nothing about any file has moved.

It covers both crates that decide a stored column. `km-song` decides the words, the flavor, the
granularity, the counts and the detected title, artist and encoding. `km-suitability` decides the
melody, the suitability and its warnings.

**`the_analysis_revision_covers_what_the_fixtures_say` fails when you need to**, and names the two
lines to edit. It hashes the tuning constants along with what the analysis says about every fixture,
so it catches tuning even where no fixture moves. What it cannot catch is a behavior no fixture
exercises. Adding one to `km_song::testing::FIXTURES` is what answers that.

### Nothing may assume cargo builds into `target/`

`target/` is cargo's *default*, not a fact. A script that spells the build output as a literal path
produces this the moment it stops being true:

```
    Finished `release` profile [optimized] target(s) in 0.23s
cmd.sh: target/release/km-pack.exe was not produced
```

**Cargo says `Finished` and the script says nothing was produced, in consecutive lines.** Both tell
the truth about different directories. **The error names the assumption instead of the fact**, which
is why this reads as a broken build rather than as a wrong path.

The directory moves for good reasons. The Docker image sets it, and another machine may point it
anywhere. **Anything that has to find a build product asks cargo**, through `dist_target_dir`. Four
things about that, three of them got wrong first:

- **Reading `${CARGO_TARGET_DIR:-target}` is not equivalent**, though it is shorter. The directory can
  also move via `[build] target-dir` in any of the config files cargo layers, and via `--target-dir`.
  **The environment variable is one of three ways and the only one that shortcut can see.**
- **It settles the excluded crate without a special case.** That crate builds into a target of its
  own, *unless* the variable is set, in which case it shares. **That is a fact about the environment
  rather than about the crate**, so neither answer could be hard-coded.
- **The value is JSON, so a Windows path arrives with its separators doubled**, and that is the
  default case rather than a corner one. Left un-escaped it produced a path to nothing. It is
  normalized to forward slashes **only when drive-lettered**, because a backslash is a legal
  character in a Unix filename.
- **It is not cached, and must not be.** Every caller writes `X="$(dist_target_dir)"`, a command
  substitution is a subshell, and a cache written inside one is gone before the assignment completes.
  **An associative array would never once return a cached answer, while its comment said it did.**
  Storing the result in the caller *is* the memoisation.

**The quiet failures are worse than the loud one.** A script that prints an empty list gives **the
wrong explanation a person will believe**. So does one that says "run the build first" straight after
a successful build.

### Every path km-admin's own pages name is a route it mounts

**km-admin serves everything under `/admin` while its two routers declare their paths without it.**
Every `src`, `action`, `hx-get` and `Location` therefore spells the prefix by hand. That is the cost
of mounting the shared page set at the same prefix the machine uses, argued in
[`docs/architecture/admin.md`](docs/architecture/admin.md). **A path that drops the prefix fails
without saying so.** An `img` falls back to its alt text, and a form post lands the browser on a page
that is not there.

So `tests/routes.rs` sweeps them. Every root-relative path in that program's templates, read from the
directory rather than a list, plus the consts that reach markup through a struct field. Each goes
through the real router under a method nothing mounts, where **`405` says the path is there and `404`
says it is not**. No handler runs, and the sweep needs no cache, no machine and no port.

**A new placeholder in a path needs a sample in `SAMPLES`**, and fails the test until it has one.
This is the inbound half of `every_call_this_program_makes_is_a_route_the_machine_mounts`, which does
the same for what that program asks of a machine.

### No test binds a non-loopback address

**A `cargo test` binary that listens on `0.0.0.0` costs a Windows developer a firewall dialog on every
rebuild, for ever.** Windows keys the rule on the *full image path*, and a test binary's path carries
a hash that changes on every relink. **The rule therefore never matches twice.** Each worktree has
its own build directory on top of that. One line left **60 inbound rules across 12 test binaries**,
most already gone from disk, before anybody traced where the dialog came from. **The prompt names a
hash and no crate.**

So **nothing under `crates/` may bind an address that is not loopback in a test.** The pattern to copy
applies it *inside the harness* rather than at each call site, so a test that forgets cannot escape
it.

The case that looks like an exception is asserting that a URL is **not** the bound address formatted.
Guarding that needs a socket whose address is not `127.0.0.1`, and **not** one the firewall cares
about. Binding `::1` is loopback on all three platforms and still not `127.0.0.1`. **Reach for `::1`
before an ignore attribute, because a test that does not run where it is written is a test that
rots.** It binds inside the Debian container too, which was checked rather than assumed.

**…and a listening TCP socket is not the only thing the firewall notices.** An mDNS daemon opens a
multicast UDP socket on every interface and gets the identical dialog. **So it looks like the TCP
case, and gets diagnosed as the TCP case, and the `bind` it sends you to read is already correct.**
A rule that says only "bind" is how the very next crate obeyed it and still asked. Its bind was
loopback from its first commit; what it left alone was the *locator*.

**So the rule has two halves. A test binds loopback, and a test gets a null locator.**

**A structural guard beats the rule.** The advertiser drops loopback addresses, so a server bound to
`127.0.0.1` produces an empty list and never reaches the daemon constructor. **Being bound to
loopback is itself the guard.** The remote's locator has no equivalent, because a remote legitimately
browses from a machine listening only on loopback. **That asymmetry is precisely where this fault
landed.**

That guard now carries more weight. A task that ticks for the life of the server calls the
advertiser, rather than one call at bind. **Make the address filter keep loopback, or move the
emptiness check after the start call, and this stops being structural.** The symptom will be a test
suite that opens multicast sockets on every developer's machine.

**One case that is latent rather than fixed**: the remote's watcher browses every twenty seconds while
offline and unpinned. Today every test injects a null locator. **A future test that ran a
default-config server for over twenty seconds would open the daemon with no `bind` anywhere in sight
to explain it.**

**`km_api::discover::watch::Watcher::start` is the second daemon constructor, and it is worse than
the first.** It holds the socket open and goes on retransmitting, rather than shutting down after a
timeout. It is split from [`Registry`](crates/machine/km-api/src/discover/watch.rs) for exactly this
reason. The registry is the merge and the policy, and has no socket in it, so **every test drives a
`Registry` and nothing constructs a `Watcher`**. A test that needs one is a test that wants a
`Registry` and has not noticed.

**Two things now enforce the multicast half rather than only the prose.** `km_api::discover::daemon`
is the single constructor, and it answers `None` when `KM_NO_MDNS` is set. `.cargo/config.toml` sets
that, so a `cargo test` binary opens no multicast socket whatever it reaches for.

`tools/dev/check-mdns.sh`, run by `task check`, asserts that function is still the only caller, and
that no test file names `Watcher::start` or `Mdns::new`. Both are a floor and not a pass. The check
matches by shape, and a variable is only as good as the one place that reads it. The standing
decision is [`KM_NO_MDNS` declines the multicast socket, and one function honours it](docs/decisions/api-and-network.md#km_no_mdns-declines-the-multicast-socket-and-one-function-honours-it).

### No test frees a port it still depends on

**An ephemeral port this process has let go is one the OS may hand to another test in the same run.**
`a_machine_that_never_answers_does_not_hold_the_attempt_open` bound `127.0.0.1:0`, dropped the
listener so the port would refuse, and asserted the client never came online. It came online — to
another test's machine, on the port this one had just released. It passed every run of its own crate
and failed in `task check`, where the whole workspace is under test at once.

**A test that needs a port nothing answers on holds the listener and never accepts from it.** The
port stays the test's, and the attempt sits in the backlog with only the client's own deadline to end
it. That is the stronger assertion anyway. **A closed port is refused at once and proves no deadline
exists**, where a listener that never answers proves one does. The fix moved that suite from 3.1s to
5.0s, which is `CONNECT_TIMEOUT` becoming reachable for the first time.

**And a wait spelled as a count of naps shrinks under exactly the load it exists to survive.** The
same run failed a second test whose budget was 600 × 10 ms. That is six seconds, against a
`CONNECT_TIMEOUT` of five and a `BACKOFF_START` of one, so a single slow handshake was the entire
margin. A budget is a deadline, and it clears one whole failed attempt.

### No test writes into the user's own config directory

**A suite that writes a per-user file rewrites the file of whoever is building.** The package
builder's recent-folders list is twelve entries in `%APPDATA%`, newest first, and the startup reopen
takes the first of them. One test called `State::begin_open`, for the folder it *refuses* rather than
the one it opens. The worker thread that call spawns records what it opened, so every `cargo test`
put a `…\Temp\km-package-builder-server-<pid>-…` folder at the top of a real list. **Eight of the
twelve entries were scratch folders and the corpus was fourth.** The reopen offered an empty temp
directory rather than a corpus somebody had been working on.

Two things make that hard to see coming. The write is **two levels below the test**, with nothing in
it mentioning the recent list. It is also **on a spawned thread**, so it lands after the test passed.

The rule is **not** "call the constructor that does not remember". That was the guard, it was written
down, and `begin_open` walked round it. **The guard belongs where the file is named.** `recent.rs`'s
`path()` answers `None` under `cfg(test)`, so a list a test holds has nowhere to save to. One line
covers every caller, present and future. `cfg!` rather than `#[cfg]`, so the production branch still
compiles and lints under test.

**The pattern to copy for a new per-user file is to be handed the directory instead.** `km-remote`
and the machine both take `--data-dir`. That is what a second instance uses, and what a test run uses
to stay out of the real one. The recent list is the one file no *flag* can name. Its question is
asked *before* a corpus is chosen, so no `--data-dir` would reach it, and it needs a guard of its
own.

**…and a `cargo test` was never the only run that is not curation.** Five dead rows accumulated in
the real list later on, and not one came from the suite. A screenshot run opens its songs folder
through the real binary. Worktree verification corpora and agent scratch folders arrive the same way:
a folder named on the command line, recorded unconditionally, before any scan.

**The guard was working, and the population it guarded was drawn too small.** So `cfg(test)` stays,
comes first, and needs nobody to remember it. `KM_PACKAGE_BUILDER_RECENT` covers the runs a `cfg`
cannot see. It names the file this run's list goes in, and set-but-empty means it keeps none.

**An integration test is one of those runs.** `tests/upload.rs` links the crate compiled *without*
`cfg(test)`. A future one that opens a folder must therefore set the variable itself, once, before
any server thread exists. An integration binary being its own process is what makes that safe. A unit
test must never do it, because `set_var` is process-wide and would race the rest of the binary.

**`KM_PACKAGE_BUILDER_PASSWORDS` is the same guard over the file that holds credentials**, and it is
the one to be strict about. A run that wrote into the recent list cost somebody a row. A run that
writes into `machine-passwords.json` is putting a machine's admin password into a store its owner
believes they control. `cfg(test)` comes first there too. `src/passwords.rs` takes the setting as an
argument rather than reading it, so its own tests can ask all three questions without `set_var`.

**…and a scratch directory is the same rule one drawer down.** A test that needs a folder takes one
from `Scratch`, which removes it on `Drop`. That covers a panic, which a removal written at the end
of a test body does not. Two shapes had escaped it, and both left directories in `%TEMP%` for good:

- **A fixed name, removed only on the way *in*.** Four `db` tests took
  `temp_dir().join("km-package-builder-kind-migration")` and cleaned it up before use rather than
  after. The directory therefore always survived the run. The name carries no process id, so **two
  checkouts running the suite at once delete each other's database mid-test**, which this repository
  arranges routinely.
- **A directory removed while something still holds it open.** The `already opening` test starts a
  real open, and its worker thread had SQLite open when `Drop` ran; Windows refuses that removal and
  does not retry. The test now waits for the job it started and closes the workspace before letting
  go. **Sixteen of these had accumulated**, one per suite run.

`Scratch` lives in `testing.rs`, in one copy. It was in four, which is how `db` came to have neither.

### A blocking call belongs on a blocking thread

Three places ran synchronous work directly on a tokio worker, and the shape is the same in all three.
**A blocked *task* costs one request; a blocked *worker* costs everything the runtime was going to do
next.** The runtime has as many workers as the box has cores, so a handful is the API, the remote and
the event stream stopping together.

The `ops` module's header said everything in it is synchronous because "the controller is a command
channel and the catalog is SQLite; neither awaits". **That premise is not quite true.** Queueing onto
an idle machine, and playing or skipping, all reach a path that opens the package. It reads the entry
and parses a song before sending anything.

**The rule to carry forward.** A catalog or controller call reached from an `async fn` goes through
`spawn_blocking`, **unless it is demonstrably only an atomic read or a channel send.** The three
above all looked like the second, and two of them were the first.

**Three more turned up in the same shape**, which is the argument for the rule rather than a footnote
to it. `play_file`, `play_upload` and `open_audition` all reach `Machine::play_path`. A debug play
therefore parses a MIDI file, opens an ffmpeg decoder, or reads both halves of an MP3+G pair, on a
tokio worker.
`open_audition` also sweeps older staging folders, a `read_dir` and a `remove_dir_all` over trees
holding whole videos, before a byte of the upload has arrived. A rule applied per call site missed
them, because these three do not *look* like catalog calls.

**Still outstanding, and worth knowing about:** every SQLite *read* is on the runtime. `search_songs`,
`get_packages`, `export_songs`, and `book::collect`, which pages the whole catalog out of SQLite
before the deliberately-off-runtime render beside it. `Library` is behind one mutex that `install`
holds for a whole transaction, so a search issued during an install parks a worker for seconds. The
fix is to move the rule from the call site to the seam. One wrapper for anything touching `Catalog`,
rather than finding these one at a time.

### A doc link is not checked, and denying `broken_intra_doc_links` was tried and taken out

**Do not add `[workspace.lints.rustdoc]` back without reading this.** It finds real stale links, about
twenty on its last run. Eight came from one rewrite, and the rest are renames nobody chased. Those
fixes are worth making by hand. The lint still cannot stay on.

**Why it cannot stay on.** Rustdoc resolves an intra-doc link against what is *nameable*, and this
codebase's prose deliberately points at things that are not:

- **A private `fn` in another module does not resolve**, even under `--document-private-items` —
  `crate::recent::path`, `crate::display::settings_wallpaper_dir`, `crate::engine::choose_instrument`.
  Satisfying the lint means widening each to `pub(crate)`, which is changing the code to suit a tool.
- **A `#[cfg(target_os = "android")]` module cannot resolve on any host that builds the docs.**
  `km-remote-android` and `km-remote-ios` document their `ffi` and `log` modules, and no CI runner
  can see either.
- **A feature-gated item vanishes with its feature**, so the check would have to name every feature.
  That includes `km-remote-core/sweep`, which is off everywhere else, and `--all-features` is out for
  the reason at the top of this file.
- Several links need `()` or a full path to disambiguate a function from a module (`arrange`,
  `known`, `path`). That is a fair thing to ask, and it was the least of it.

So the lint's price is a bespoke feature list, two crates excluded, and a handful of private items
made `pub(crate)` for documentation's sake. **The drift it catches is real but slow.** A reader
following a link catches a rename that breaks one, and this repository has readers. Anybody trying it
again changes the *policy* first, by deciding that a doc link may only name a public item.

### A poisoned lock is recovered, never panicked on and never silently skipped

**`unwrap_or_else(|poisoned| poisoned.into_inner())`, everywhere, with no exceptions.** A panic that
happened to be holding a lock does not make the `String`, `u64`, `Connection` or `HashMap` behind it
untrustworthy. What it costs is the one request it happened in. Turning that into a permanent
failure trades a bad minute for an application that, in a window with no console, is silently dead.

**Four shapes, and each of the three wrong ones fails in a different direction:**

| shape | where | what it did |
|---|---|---|
| `into_inner()` | `machine.rs`, `km-package-builder/server.rs`, `km-video` | correct |
| `.expect("… is never poisoned")` | `km-remote-core` ×17, `scan.rs` ×9, `km-api/discover/watch.rs` ×5 | one transient fault killed that surface for the life of the process |
| `if let Ok(mut slot) = …` | `km-api/auth.rs`, `km-api/server.rs` | dropped the write and returned as though it had happened — a password change and a sign-out-everywhere both reported success while changing nothing |
| `.ok()` / `.unwrap_or(0)` | `km-api/auth.rs` | **failed open.** A token is an HMAC over the password hash and the session epoch, so an `epoch()` degrading to `0` would let tokens minted before the last sign-out start verifying again |

The sharpest case was one lock with two policies. `km-package-builder`'s single `Arc<Mutex<Db>>` was
recovered in `server.rs` and panicked on at nine sites in `scan.rs`. A handler that panicked while
holding it therefore left the web UI working and the next corpus scan dying. That was nobody's
decision: it was two files not knowing about each other.

**Where a type takes the same lock more than twice, give it one `fn locked(&self)`** rather than
repeating the recovery at each call site. `db::locked`, `AdminAuth::write_hash`,
`AdminAuth::lock_attempts`, `Machine::lock_state`. One place to read the policy, and one place it can
be got wrong.

### A filter written in `dispatch` has to be written in `seek_ticks` too

`Sequencer::dispatch` is where a per-song correction lands: a bank select dropped, a program
substituted, a channel silenced. **`seek_ticks` does not go through it.** It scans the event list
itself, fills last-value-wins tables of controllers and programs, and emits into the sink directly. A
filter written in one place and not the other is a filter that works until somebody seeks.

The failure is quiet, and it is not confined to the seek. A song played straight through sounds
corrected. The first seek re-establishes the bank select the fix removed. The *rest of the song* then
plays the drum kit the correction existed to prevent. Switching the SoundFont mid-song replays
through the same path, so it has the same shape.

**The rule to carry forward:** anything that changes what `dispatch` emits is two edits, and the
second one is in the replay loops. `a_seek_does_not_restore_a_suppressed_bank_select` is the
assertion. `a_seek_replays_a_bank_select_that_is_not_suppressed` sits beside it, so an absence is
known to be caused rather than vacuous: an absence asserted with nothing producing it passes for ever.

The two sweeps over `km_song::testing::FIXTURES` that already compare a played sequencer against a
seeked one are the pattern to extend rather than to copy. They assert *state*, which is the thing a
listener would notice.

### `CARGO_TARGET_DIR` is not set, and should not be

Moving cargo's output out of the checkout into per-checkout slots costs more than it buys. That
arrangement was tried here and removed, and each of these is a thing somebody will otherwise
re-derive:

- **Two checkouts pointed at one build directory share *artifacts*, not just cargo's lock.** Cargo
  hands a cached test binary compiled in checkout A to a run in checkout B whenever the sources
  match. That binary carries checkout A's manifest directory. It surfaced as nine tests failing on a
  fixture path under **another worktree's** directory, one that by then did not exist. **It reads
  exactly like somebody else broke master.**
- **The per-worktree slot could not work for the case it was written for.** It arrived as a rewrite
  of a settings file whose environment block resolves **when a session starts**. Entering a worktree
  relocates a session that is already running, so it keeps the parent's value and nothing ever writes
  to the fresh slot. Measured: two slots created and never used, while a third collected builds run
  from inside them.
- **Nothing inside a checkout can repair that.** A real `CARGO_TARGET_DIR` beats every configuration
  file, verified in a scratch crate. `[build] target-dir` loses, and so does an `[env]` entry with
  `force = true`, because *cargo itself* reads the variable before it applies that table. **This is
  the second cargo-config assumption this project has had to disprove by experiment**, the first
  being that `include` works on stable.
- **Removing worktrees left the slots behind**, so most of them were orphans of worktrees that no
  longer existed, several GB apiece. An in-tree `target/` leaves when its worktree does.

**What the removal cost in code: nothing** — which is the `dist_target_dir` rule earning its keep.

## Risks to keep visible

- **Every content file is from a stranger, and three of the things that read one are not this
  project's code.** The trust boundary is the file, not the network. People who have never met trade
  a package, a wallpaper pack, a SoundFont bank, a video and a `.kmbuild`. The machine takes a
  package the operating system hands it **before anybody has typed a password**.

  What follows from that is written down where it is enforced. `is_safe_path` and `is_safe_name` in
  `km_kmpkg`, `Db::best_file`'s containment, the `MAX_*_BYTES` ceilings, and `Frame::fill_from`
  asking the decoded picture rather than the container. **The Rust is the defensible half.**
  `unsafe_code` is denied workspace-wide, and the one allowance is console detection. Nothing that
  parses a file is unsafe, so the failures worth hunting are traversal and exhaustion rather than
  corruption.

  **The C libraries are the other half, and are not defensible from here.** A hostile video reaches
  libavformat and libavcodec, a large C codebase with a steady supply of advisories and no mitigation
  this crate can apply. The answer is the ffmpeg version, not the code around it. **There is no
  fuzzing.** The two cheapest targets are `km_song::Song::parse` and `km_cdg::GraphicsStream::stats`,
  both of which take `&[u8]` directly, and both are five lines if somebody wants the coverage.
- **`rustysynth` is a git dependency on a fork, so it sits outside advisory coverage**, and a `.sf2`
  is a file a stranger sends. What holds it still is `Cargo.lock` plus `--locked` on every release
  path, rather than a `rev` in the manifest. The reasoning for that sits beside the dependency
  itself.

  What is worth knowing here is the part an advisory feed would otherwise have told you. **The fork's
  SF2 reader is visibly more careful than upstream's.** `try_reserve_exact` rather than
  `with_capacity`, so a chunk header claiming four gibibytes is an error and not an abort. Chunked
  reads. Every sample offset checked against the wave data before the oscillator interpolates past
  it. **A `cargo update` moving the branch is the event to look at**, because nothing else would
  report it.

- **The wallpaper tool's cache holds thousands of stock originals, and some providers' terms forbid
  systematic mass downloads.** **Nothing is being done about it, and this entry is the record of
  that.** It is a local cache, gitignored and never distributed, and deleting it would not make a
  past download un-happen. It is listed rather than tasked because it is a position taken knowingly.
  A later reader who finds the clause should not have to work out whether anybody had noticed. What
  *is* acted on is distribution.
- **An edit of a large package is O(archive), and the disk needs twice its size while it runs.** A
  byte copy on a curation workstation rather than on the machine, accepted knowingly. **Two things to
  know before treating a slow edit as a bug.** It is expected, and the temp-and-rename means a 20 GB
  package needs 40 GB free.
- **Format probing over custom I/O depends on the entry's name.** If the naming scheme is ever
  changed, **the extension is the part that has to survive.**
- **The CD+G "skip and carry on" rule is load-bearing and will look like sloppiness to somebody
  later.** The corpus proves it twice over. **The temptation to tighten it up is a temptation to
  reject songs that work.**
- **A package built before the language work carries a raw header value, and that is fine.** It opens,
  installs, searches and plays exactly as it always did. **The risk to watch is somebody deciding to
  enforce the code where it would be enforced against every machine in service.**
- **Video packaging depends on an ffmpeg binary this repository does not ship.** A runtime dependency
  of a *tool*, discovered when a re-encode is attempted rather than at startup. A folder of files
  already in profile needs no ffmpeg at all.
- **The curation database's `songs` table is rewritten once**, and it is the only migration in that
  tool that is not an `ADD COLUMN`. **A mistake there would destroy a corpus's worth of hand curation
  rather than merely fail.** Two tests cover it, and the column list is derived rather than written
  down. Worth a backup before the first open of a large corpus all the same.
- **ffmpeg is the only C dependency outside SDL that needs *two* things to build.** `cargo build -p
  karaokemachine` must go on needing neither — **worth checking, because it is the kind of property
  that erodes silently.**
- **Disk, for a real video catalog.** Not a constraint on the appliance, which has room twenty times
  over. It stays listed because the *packaging* machine holds two copies while it works, **and that is
  a different machine with a different disk.**
- **Lyric-format variance** is the top risk. The mitigation is the dump tool and a fixture corpus
  grown from real files as they break things.
- **Melody-detection false positives are worse than no detection**, because a wrongly muted channel
  ruins a song. The confidence gate, and the test that asserts *abstention* on ambiguous files, are
  the guard.
- **Suitability score credibility.** The rubric is a heuristic. The breakdown and warnings are stored
  rather than only the number, so a low one is explainable and the weights can be revised.
- **SoundFont licensing — settled**, with each bank's own terms recorded beside it in the table. The
  bundled bank's license asks that the download not be hot-linked, **which is why the fetch script
  caches per machine rather than downloading per build.** **Font licensing is still open** — the
  display borrows a system font and none is bundled.
- **A second song format has been investigated and not adopted.** Findings are in
  [`docs/research/st3.md`](docs/research/st3.md); it stays an open question, deliberately not a plan
  item.

## Verification, end to end

```sh
cargo km-test
cargo km-lint
cargo run -p km-song --features testing --example write_fixtures
cargo run -p km-lyrics -- dump fixtures/generated/soft_karaoke_header_on_words_track.kar
cargo run -p km-audio --example render_wav -- fixtures/generated/…kar out.wav   # no device needed
cargo run -p km-pack -- spec ./songs --out vol1.kmspec.yaml
cargo run -p km-pack -- build vol1.kmspec.yaml
cargo run -p km-pack -- check vol1.kmpkg
cargo run -p karaokemachine -- --show-paths
cargo run -p karaokemachine -- --data-dir /tmp/km --headless
cargo run -p karaokemachine -- --set-password hunter2
cargo run -p km-package-builder -- "<your karaoke folder>" --init --scan --open

curl localhost:8177/api/v1/discover
curl 'localhost:8177/api/v1/songs?q=beatles&min_suitability=6'
curl -XPOST localhost:8177/api/v1/queue -d '{"number":10234}'
websocat ws://localhost:8177/api/v1/events
dns-sd -B _karaokemachine._tcp     # or avahi-browse; confirm the advert
open http://localhost:8177/           # the singer's remote
open http://localhost:8177/admin/     # the owner's page
open http://localhost:8177/dev/       # only for a run started with --dev-remote
```

**Manual checks that matter and cannot be automated:**

- the syllable highlight lands on the beat at 1.0×, 0.75× and 1.25× tempo;
- transpose ±6 leaves drums unchanged;
- the melody toggle affects only the declared channel, and leaves no stuck notes;
- skip mid-song silences all sounding notes;
- the queue advances automatically on song end;
- wallpapers crossfade without stutter, and lyrics stay readable over bright images;
- the connect panel shows a URL that works from a phone;
- **and a truthful message instead** when the machine is bound to loopback, offline, or failed to
  start;
- a phone on the same LAN finds the machine by QR, by typing the shown URL, and by mDNS.

**Two more need two devices, and they are the offline remote's.** A favorites folder shown on one
phone's screen and read by another's camera lands in the folder the reader was standing in, and adds
only. The code is never on screen while another camera reads it, and a code from a differently named
folder says so before anything is written. A backup saved from one device restores onto a second with
nothing removed, and twice in a row adds nothing the second time.

On Android the camera cases only a device shows are worth walking. Granted, denied, **denied twice**,
and the prompt dismissed by a tap outside. Denied twice stops the prompt for good, so the page must
say the camera was refused and name Settings. Each case must end in a sentence rather than in
"Starting the camera…".
