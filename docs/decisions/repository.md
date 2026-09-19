# The repository itself

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Where a wallpaper pack's photographs may come from

**A pack that is passed on carries only images whose license grants redistribution — CC0, Public
Domain Mark or CC BY — and nothing built from Pixabay or Pexels is ever committed or staged.**

Those two providers grant use and not redistribution, and both name a wallpaper pack among the forms
that stops at. A pack built from either is therefore for the machine that built it, which is what
the tool is for and what `--zip-dest` defaults to.

**Openverse is the source that satisfies this**: an aggregator over Wikimedia Commons, Flickr,
StockSnap, rawpixel and museum open collections, asked for `cc0,pdm,by` and checked again on the way
back, because a provider is not trusted to honor its own filter. `category=photograph` is on by
default and is not a size optimization — it excludes `digitized_artwork`, which is what the archival
scans rejected by eye from the shipped set turned out to be. It needs no key to work and wants one
anyway: **anonymous is capped three ways, not one** — 200 requests a day, 20 results a page, and no
search able to see past its first 240 hits, which is a ceiling rather than a delay.

**Both other providers stay**, because a pack somebody builds with their own keys for their own
machine is exactly what the tool is for; deleting them would strand existing caches and throw away
the quota and throttle knowledge in those two files, to solve by removal a problem better solved by
making the constraint visible in data.

**The license is a property of the image, not of the site.** One `&'static str` per provider makes
the question invisible, and an aggregator cannot use it at all. The allow-list is a **constant in the
code and never a config key**, so that widening it is a change somebody reviews rather than a setting
somebody types.

`by-nd` is out because this pipeline crops, blurs and re-encodes, so what it produces is a derivative
and No-Derivatives does not cover one. `by-sa` because explaining which of two licenses governs which
bytes of one download is a cost with no matching benefit, and it is what puts Wiki Loves
Monuments, the best body of freely-licensed landmark photography, out of reach if the shipped set
ever wants landmarks. `by-nc` because it would bind everyone downstream of an otherwise
permissive application.

CC BY costs a credit **and a statement that changes were made**, which every image earns and which
`ATTRIBUTION.md` therefore says once at the top rather than on all 120 lines.

## Local assets in a checkout

**`local/assets/` overlays `assets/`, per path, and only when the machine is run from a checkout.**

`assets/` is the *shipped* tree — `dist_stage_assets` copies every file in it into the Windows
folder, the macOS bundle and the tarball, `tools/port/machine/android/assets.sh` into the APK,
`installer.iss` recursively, and cargo-deb by glob — so anything a developer puts there for local
testing goes into a release. A built wallpaper pack landing in `assets/wallpapers/` travels into six
carriers, tens of megabytes of somebody else's photographs, with only the APK's size warning ever
mentioning it.

**The rule covers wallpapers.** Nothing writes a SoundFont bank here, because a bank put here is
inaudible to every build except a `cargo run` — see `Switching the SoundFont locally`. A hand-placed
`gm.sf2` still wins.

**`/local/` is gitignored wholesale**, so nothing here can be committed by accident.

**Resolved by rule and never by a setting**, on the argument `Where packages live` makes: a second
way to *say* where assets are is a second thing that can disagree with the first. Two settings name a
file somewhere else and both beat this overlay outright — `wallpaper.dir` names a folder, and
`display.font` names a file and is in scope of
[`Only debug. names a file`](foundations.md#only-debug-names-a-file). `audio.soundfont` is not one of
them: it holds a bank **id** resolved against the SoundFont folder, so it names no file anywhere — see
[`The bank is chosen by id, not by path`](audio.md#the-bank-is-chosen-by-id-not-by-path).

**An installed build cannot reach it, structurally rather than by a check somebody has to remember**:
every carrier lands `assets/` beside the executable or in `Contents/Resources`, and only the
working-directory branch of `discover_asset_dir` can carry an overlay at all. `rooted_at` never
enables one, which is the sharp part — it is what the tests use, and they run with the workspace root
as their working directory, so an overlay computed there would have the SoundFont-resolution test read
a developer's own 206 MiB bank: red on their box and green on CI, which is the worst failure shape
available. The one production caller that really is in a checkout asks by name, so
`--data-dir ./local/km-<name>` keeps its overlay.

**Per path, not per tree**, so an overlay holding only a bank leaves the bundled font and wallpapers
alone — with one accepted cost, stated because nothing on screen says it: a `local/assets/wallpapers/`
**replaces** the bundled folder rather than adding to it, so the four committed gradients are not
shown while a pack is sitting there. `WallpaperConfig::dir` is one path, and merging the candidates is
refused; the thing that actually wanted expressing — extra individual files — is a second field. See
`The wallpaper folder is chosen again, not once` in [`interface.md`](interface.md).

**Nothing deletes the folder** — `task clean` is about `dist/`, and a command that removed a 206 MiB
download, every worktree's data directory and somebody's notes would be a worse bug than the one this
fixes. Visibility replaces a deleter: `--show-paths` names the overlay while it is in use, the machine
says so once at startup, and `rm -rf local/assets` is the whole undo.

## Switching the SoundFont locally

**`audio.soundfont` in `settings.json`, written by `--set-soundfont` and undone by
`--clear-soundfont`; `task soundfont BANK=<name>` is the command.** The flag takes a path, **installs**
the bank into the machine's own SoundFont folder — a hard link where the filesystem allows one — and
writes its **id**. The folder is what says which banks exist, so a bank left only in the shared
download cache would be named and then not found.

This is the only way to reach a bank the table knows how to *fetch*.
[`Choosing a bank`](audio.md#choosing-a-bank) writes the same key from a phone, over the same level
protocol, for a bank already on the machine.

**One door, and the door has to be the one every build can see.** `settings.json` is per-machine, is
in no carrier, and is not committed, so this is local by construction rather than by care — which is
the property `assets/` cannot have and `dist/` cannot have either. A bank in `local/assets/soundfont/`
is audible to `cargo run` and inaudible to `dist/bin`, to the `.deb`, to the bundle and to the APK,
with nothing anywhere saying so.

**The bank is opened before the key is written**, through `km_audio::Bank::load`, and a file that will
not play is refused with the synthesizer's own words. The symptom of naming a bank that will not load
is a machine that comes up on a sine test tone with the reason in a log nobody reads. Opening it also
means [a bank that loaded incomplete says so](audio.md#a-partly-loaded-bank-plays-and-says-so) here
too. That check is what makes this a flag rather than a line somebody edits into the file by hand.

**The level travels with the bank**, because several banks exceed full scale at `1.0`; whatever
`music_volume` was is stashed in `soundfont-override.json` beside the settings file — *beside* and not
*inside*, on this row's own "a second way to say something is a second thing that can disagree"
argument, since it is a note the command left itself and not a setting anybody sets. Switching from a
bank that needed `0.8` to one that needs nothing goes back to the owner's own level rather than
leaving the first bank's reduction behind, and a level edited by hand while an override was in force
is kept rather than overwritten, on both the set and the clear path.

**Sixty-three banks are named and sixty-two can be fetched.**
`crates/machine/km-banks/data/soundfont-banks.conf` is the table and `tools/setup/soundfont-banks.sh`
reads it, and each row was confirmed against the research note's measured file size before it was
written down. Fifty-five of the sixty-two are pinned by archive.org's own published sha1 rather than a
computed sha256 — a digest's job here is to make a corrupted or silently replaced download fail
loudly, which either does. Arachno is `manual` and always will be: its terms forbid reproduction, so
there is nothing this project may mirror.

## Choosing the debug banks from a list

**`task soundfont:debug:choose` ticks any of the sixty-three into `Ctrl+2`…`Ctrl+9`, and fetches
what is ticked and missing.** The command it sits beside can only reach the top of the table: it
keeps the first eight rows already cached, and the table is in rank order, so the slots are
structurally the highest-ranked cached banks. The other fifty-four are otherwise reachable only by
typing eight `<path>=<name>=<volume>` specs whose paths are cache filenames like
`41.8mg_saphyr_two_thousand_gm_gs_bank.sf2`. **The writing, the checking and the switching are
untouched.**

**The list is `tools/dev/km-pick`, which knows nothing about SoundFonts.** It reads
`<key>\t<flags>\t<label>` rows from a file, wraps `inquire`'s `MultiSelect`, and writes the chosen keys
to a file. Everything about banks stays in `soundfont-debug.sh`, so `banks.rs` and
`soundfont-banks.sh` remain the table's only two readers — a third one in Rust, outside the machine, is
exactly the drift the one-file-two-readers arrangement exists to prevent.

**The rows come from a file and never a pipe.** Piping them in is eaten on Windows: crossterm reads
`STD_INPUT_HANDLE` there, not `CONIN$`, so the row text arrives as keystrokes — the spaces in the
labels tick whatever rows the cursor is on, a newline confirms the lot, and seven banks nobody chose
come back in about a tenth of a second with no error anywhere. Unix opens `/dev/tty` and would be
fine, which is exactly why it is worth getting wrong: the wrong design works perfectly on two of the
three platforms this ships on. **stdin belongs to the prompt**, and the no-terminal guard asks about
stdin rather than stderr for the same reason.

**Nothing unavailable can be ticked, and refusing happens at the prompt rather than after it.** A
`manual` bank that is not already on the box is drawn, so that it exists and its reason are visible,
but marked so that confirming with it ticked is refused with the list still up and every other tick
intact. Accepting and then rejecting on the way out throws away seven good decisions to report one bad
one, which generalises past this case: a picker that can only fail by exiting turns any single mistake
into the loss of the whole answer.

**Not a flag on the machine, and the reason is where the candidate list comes from.** It is the asset
cache, `KM_SF2_DIRS` and `tools/setup/fetch-assets.sh` — all shell, all development-box concerns, and
`fetch-assets.sh` is the only downloader in the repository and the one place that knows the pinned
digests and the two banks published inside a zip. The machine's own fetcher writes into
`data_dir/soundfonts/`, which is a different folder answering a different question. So the only part
that has to be Rust is the checkbox list, and only that part is.

**One flag on the machine, and it is the smallest one that works:** `--show-debug-soundfonts` prints
each slot in exactly the form `--set-debug-soundfonts` reads. The pair round-trip, so the picker can
open with the current slots ticked — the difference between choosing a list and re-choosing one from
scratch — without a shell script parsing `settings.json`, and there is no `jq` on the development box
by policy.

**Slot order is the table's, not the order things were ticked.** The highest-ranked bank chosen is
`Ctrl+2`, which is the rule the plain command already uses, so the two cannot disagree about what a
slot means. Ticking order is invisible on a checkbox list anyway, so making it significant would be
making a hidden thing load-bearing.

**Fetching on confirm is inside what [`Nothing downloads`](#nothing-downloads) permits**: it happens
only on an explicit request naming banks, only from the pinned URLs with the pinned digests, one at a
time, and never on any path a song takes. What is added is that the request can name several at once —
so it says which ones and how much, and asks, before a byte moves. Some rows are over a gigabyte,
which is reason enough. A `manual` bank is refused by name with its page, exactly as `task soundfont`
refuses one.

**[inferred]** **`inquire` is the one dependency, and `km-pick` is a workspace member rather than an
exclusion.** `tools/cmd/assets/km-wallpaper-pack` is excluded for a specific reason — a `reqwest` feature
unification that made a tool's own build depend on how the workspace was built — which does not apply
to a terminal prompt. Membership costs a few seconds of build and buys `cargo km-lint` and
`cargo km-test` coverage; `ALL_APPS` in `tools/dist/bin.sh` is an explicit list, so nothing new is
staged into a release. The known risk is Windows: crossterm drives the console API and Git Bash's
mintty is a pty rather than a console, so the picker may need Windows Terminal. It says so and exits
rather than failing obscurely.

## Which banks the machine offers

**Nine, ranked, and the table holds sixty-three.** `rank` in `soundfont-banks.conf` is the field that
separates the two: a ranked row is the shortlist `/dev/` opens on and offers a button for, an unranked
one is behind `?all=true` there and is otherwise reached from a shell — `task soundfont BANK=<id>`, or
the `Ctrl+1`…`Ctrl+9` switcher. Rank 1 is the bundled bank, so the shortlist offers eight buttons.

**Nine because the switcher has nine keys and a phone has one screen** — not because the measurements
imply a cut there. Ranks 2 to 9 are a judgment and are meant to be revised; a rank is one line.

**A catalog is not a shortlist.** The survey measured sixty-eight banks and found sources for
sixty-two of them. A phone is not where somebody browses a catalog, and a list of sixty rows with
terms printed on each is not a choice, it is a scroll.

**What the unranked rows are for, and why they are in the shipped binary at all.** They are the
field the next person will want when they ask "is there something better than what ships?", and the
honest answer to that question is otherwise "nobody could tell you without downloading four
gigabytes first". A bank nobody can fetch is a bank nobody will check. The whole table is about 60
KB of text compiled in, which is a cheap price for not making somebody repeat a survey.

**The API is not gated by rank, deliberately.** `Machine::fetch_soundfont` looks a bank up in the whole
catalog, so an explicit request naming an unranked bank is honored — that is exactly the thing
`Nothing downloads` permits, and refusing it would make the shortlist a restriction rather than a
curation. What `rank` governs is what is *offered*: what a singer is shown without asking. The route is
behind the admin password, like every other write on this surface.

**And the list is not gated by rank either.** `GET /audio/soundfonts?all=true` answers with the
whole catalog instead of the shortlist, every row marked `offered` or not. The default answer is
nine rows, which is the half that matters. Naming what you will already do on request is not a wider
permission, and treating it as one makes the shortlist a restriction by the back door. `audio.read`
at either width, because nothing here is a change to the machine.

**The caller is the `/dev/` page, and that is the whole of the intended audience.** It is the page for
somebody working on the machine rather than singing at it — the same split `Switching the bank while it
plays` in [`audio.md`](audio.md) makes with `Ctrl+1`…`Ctrl+9`. The argument against sixty rows on a
phone is unaffected by their being fetchable over HTTP.

**The nine are ranked by ear, not by the metric.**
§14 of the research note is somebody comparing all nine on a real machine, and song-to-song spread —
the number this project chose the bundled bank on — ranks their first choice **seventh** and their
second **ninth**. The bank spread ranks first came seventh by ear. So the order is a person's, and the
measurements are what got the field down from sixty-eight to nine rather than what sorted the nine.

Reputation does no better: SGM-V2.01 and SONiVOX GS250 are the banks the forums name most often and
they measure 13.0 and 10.0 LU against the bundled bank's 7.8.

**One bank is marked `Recommended`, and its terms are printed on the same row**, as they are on every
other row. A badge is a suggestion about how a bank *sounds*; it replaces nothing else the row says,
which is why it is a mark on a full row rather than a shorter list.

**Exactly one, and a test holds it.** A second recommendation is not a stronger one; it is a list, and
the other eight rows are already that. It is a separate field from `rank` because they answer different
questions — an ordering, and a suggestion — and the recommended bank being a 261.9 MiB download is
exactly the case where a future list might reasonably put something smaller first while still
recommending it.

## Which source `km-admin` opens on, and what a key of your own changes

**Openverse is the default and needs no account; Pixabay and Pexels appear only once somebody has
pasted their own key, with the provider's own clause printed above the field.** This is
`Where a wallpaper pack's photographs may come from` applied to a program that ships to people rather
than to a build-time tool, and the difference matters enough to be its own row.

**Because shipping a program whose job is fetching wallpapers is exactly what one of those clauses
names.** Pexels' API guidelines bar *"making Pexels content available as a wallpaper app"*; Pixabay's
license bars distributing content *"on a Standalone basis"* and its terms name **wallpaper** among the
forms. A command line run from a checkout can be left where that decision leaves it: a pack built with
somebody's own keys for their own machine, no distribution occurring. A downloadable program with a
search box in it is closer to the thing being described, and pretending otherwise would be reading a
clause hopefully.

**What keeps it on the right side is that the program is not the content, and every key is the user's
own.** This project mirrors nothing, re-serves nothing and ships no photograph it did not build under a
license that grants it. Somebody using their own Pixabay key, under their own agreement with Pixabay,
to put pictures on their own machine is doing the thing that license permits — and the program's part
in it is a search box and a contrast meter.

**Three things make that structural rather than a claim.** Openverse is what the page opens on and the
only one that works out of the box, so the path of least resistance is the one whose packs may be
passed on. The other two are inert until a key is typed — there is no shared key, no fallback and
nothing to forget to remove. And **the pack says which kind it is**, from `Manifest::redistributable`,
which is derived per image rather than per site: a pack from Pixabay is marked *for the machine that
built it*, on the page, next to the result.

**Deleting the two providers is not better.** It would strand every cache built with them, throw
away the quota and throttle knowledge in those two files, and solve by removal a problem better
solved by making the constraint visible.

## Where a key somebody typed into a page lives

**In memory by default; in `km-admin`'s own data folder only if the box is ticked; never in a
document.** `provider-keys.json`, beside the settings and never inside them.

**`km-wallpaper-pack`'s rule cannot be kept unchanged here.** That tool takes keys from the
environment and treats a key-shaped entry in its `config.toml` as a hard error, so that a config
file is always safe to commit or send to somebody. The half that matters — *a key never goes near a
document anybody would share* — survives here intact. The half that cannot is the environment: a
person typing a key into a form has none to put it in, and `std::env::set_var` is `unsafe` in
edition 2024 against a workspace that denies `unsafe`.

**Two files rather than one**, which is the shape of the promise: `settings.json` holds search terms
and thresholds, `provider-keys.json` holds credentials and nothing else. So "forget my key" cannot take
somebody's search terms with it, and "remember my terms" cannot quietly write a credential. Forgetting
deletes the file rather than emptying it — a `{}` left behind reads as *a key is remembered* to
anybody who finds it.

**The environment still works and is consulted first**, so somebody who already has `PIXABAY_API_KEY`
set never types anything. A remembered key beats it, because it is the more recent statement of intent
and the one they can see.

**0600 where the platform has it, and the page says so where it does not.** On unix the file is
owner-only. Windows has no one-line equivalent this program can apply, so the sentence beside the
checkbox changes to say that the file is protected by the profile directory and nothing more. Refusing
to remember a key at all on Windows would trade a real convenience for a protection that was not going
to be provided either way; **claiming** the protection would be worse than both.

## The bank table is a crate, because it has a reader the machine cannot give it

**`crates/machine/km-banks` holds the table, its parser and the rules for checking a bank that
arrives.**

**There are two fetchers, and two obvious routes are closed to the second.**
`tools/cmd/assets/km-admin` downloads a bank on a desktop and uploads it, which is what serves a
machine with no internet and a television box with no shell. It cannot depend on `karaokemachine`,
which links SDL3, an audio device and a renderer to reach a 42 KB text file. And it cannot ask the
machine, because `GET /audio/soundfonts?all=true` answers with `SoundFontOfferDto`, which carries a
bank's size, license, note and whether it is *fetchable* — and deliberately **not its URL or its
digest**. That omission is correct on its own terms (a phone has no use for a download address) and
it is exactly what a downloader needs, so the table has to live somewhere both readers can reach
rather than the DTO growing two fields for one caller.

**One definition, three readers** — the shell script, the machine and the desktop tool. A second copy
of a digest is a download that verifies against the wrong number.

**The rules for checking a download live with the table and the transfer loop does not**, and the line
between them is the useful part. Which hash a row is pinned with, how far past the stated size a body
may be before it is refused unread, how a bank published inside a zip is taken out, what a half-finished
download is called: those are properties of the table's own `digest`, `bytes` and `archive` fields, and
they are stated once in `km_banks::digest`. How big a bite a loop takes out of a socket is not — the
machine reads 64 KiB at a time over `ureq` on a thread beside the audio thread, `km-admin` streams over
`reqwest` on tokio, and the two differ in the places that decide a transfer loop's shape: cancellation,
and where the progress counter lives. **Two transfer loops is honest; two digest rules would not be.**

**The machine re-exports it under the name it has locally.** `use km_banks as banks;` in `lib.rs`, so
every `crate::banks::…` call site is untouched.

**Its dependency list is `sha1`, `sha2` and `zip`, and keeping it that short is a requirement rather
than a preference.** `km-admin` lives outside the workspace precisely so that its TLS stack cannot
unify with `km-package-builder`'s absence of one; a leaf crate reaching across that boundary must
therefore carry nothing that could re-open the question. All three are pure Rust with no TLS and no C.
See the `exclude` note in the root manifest.

## Where a bank may be fetched from

**Every bank is fetched from the publisher's own address, pinned by a digest, and none of them is
mirrored from here.** `assets/` holds exactly one bank, which is the bundled one.

**Every row prints what its own file says about its terms.** §8 of the bank survey read that out of
each file's own bytes, and `license` carries the finding beside the name rather than behind anything:
it is one of the handful of things somebody is choosing between sixty-odd banks on, alongside the
size, the loudness spread and the one-line note.

**A row with no direct address gets no button, and that is not the same as hiding it.** Arachno is
`manual`: arachnosoft.com serves the file through a page rather than a URL, so there is nothing to
pin a digest against and nothing a program can fetch. The row is shown with a link to the page that
does publish it.

**Fifty-one of the rows come from a single archive.org item, and that item was checked rather than
trusted.** Three of its files match rows already confirmed against the research note *exactly* —
`FluidR3_GM2-2.SF2` is the byte count `fluidr3` names as its zip member, `SGM-V2.01.sf2` carries the
identical sha1 `sgm` is pinned to, and `TimGM6mb.sf2` is `timgm6mb`'s. Every one of its fifty-four
files was fetched and verified against the manifest, and none mismatched, so the item is a faithful
copy of what it claims to hold. That is what the digest pins are for.

## Nothing downloads

**Nothing is fetched for a song, and nothing is fetched unasked.** The appliance may have no internet
and must never depend on one; packages reference files the owner already has; nothing downloads on a
timer or because a song needs it. A machine that never has a network never reaches the code that opens
a socket.

**The product does reach the network, at run time, on a machine somebody is standing in front of**:
the machine fetches a SoundFont bank when somebody asks it for one by name. Four constraints hold that
where it is:

- **Only on an explicit request naming one bank.** There is no "check for updates", no background
  refresh, and no auto-updater — adding one reopens this row.
- **Only from a pinned URL with a pinned digest**, out of a table compiled into the binary. A bank
  whose bytes do not match is deleted and reported; nothing unverified is ever installed.
- **A failure is a message, not a fault.** The machine goes on playing whatever it was playing.
- **One at a time**, on a thread of its own, and never on any path a song takes.

The alternative is already possible and already worse. An owner can fetch a bank at a shell with
`task soundfont BANK=…` and drop it in a folder — so the bytes arrive on the machine either way,
from the same URLs, on the same terms. What the shell requirement buys is not safety but the
exclusion of everybody who does not have a shell, on a product whose whole design is that it is
operated from a phone. See [`Choosing a bank`](audio.md#choosing-a-bank), and
[`Where a bank may be fetched from`](#where-a-bank-may-be-fetched-from) for what may be offered.

**A first start can fetch one bank**, because the setup programs offer a tick box and the machine
carries it out. Who is asking is unchanged — a person ticked a box naming one bank, on a page that
printed its size and its terms beside it — and what changed is that the program which would have been
asked was not running at the time, so the answer reaches it as a file. See
[`Offering the recommended bank at install time`](distribution.md#offering-the-recommended-bank-at-install-time).
Three things keep that inside the four constraints:

- **No file, no code path.** The request exists only where somebody ticked the box, and a machine whose
  owner did not never reaches `crates/machine/karaokemachine/src/fetch.rs` at all.
- **Three starts and then it stops.** A first boot before the network is up is the case this exists for
  — an appliance carried to a television has often not been given a password yet — and a request that
  never expired would be the "background refresh" the first constraint rules out, wearing a retry's
  clothes.
- **The installers still fetch nothing.** They write two lines of JSON. `What setup fetches` in
  [`distribution.md`](distribution.md#what-setup-fetches) is unchanged, and that is the point of doing
  it this way rather than downloading during the install.

**An owner's own choice wins over the tick box**, which is the other half of "nothing is fetched
unasked": if `audio.soundfont` is already set the request is dropped unread, so a repair or upgrade
install cannot talk over a bank somebody chose.

**The line is between what a person asks for and what a machine decides to do on its own.** Two other
things sit on the asked-for side of it:

- **`tools/setup/fetch-assets.sh` and the wallpaper pack's `fetch` phase go and get things when a
  person runs them**, and never on their own. Both are the packager's side of the line: what they
  bring back is material somebody chose, on terms that are that person's to accept.
- **The Windows installer fetches Microsoft's WebView2 bootstrapper when it is absent, and nothing
  else, ever** — `What setup fetches` in [`distribution.md`](distribution.md).

## What the README may show of a catalog

**Real songs, from the owner's own corpus: real titles, real artists, and two real lyric lines on the
playing screen.** The alternative is the repository's own fixture, and a picture captioned
`Twinkle Twinkle — The Test Fixtures` advertises a demo rather than a product. The remote's list is the
clearest case: a column of `Song 1001 / Even Artist` — which is exactly what the `testing` fixtures
produce — tells a reader the thing has never been used on anything.

What gets published is a *screenshot of* a catalog, not the catalog: titles and artists are close to
facts, and the words on the playing screen are a few caught mid-syllable, which is what a karaoke
machine's own screenshot inescapably is.

**Two lines, in twice the pictures.** `hero_tick` in
`crates/playback/km-display/examples/screenshots.rs` requires `frame.lines.len() >= 2` and refuses a
song with fewer, so `screen-playing.png` carries the line being sung *and* the one after it, and
`screen-queue.png` carries the same two dimmed behind the overlay — one of them being the site's
hero and its `og:image`, the copy re-hosted by everything that unfurls a link. A screenshot of a
karaoke machine with no words on the screen shows nothing of what the machine does, and two lines
mid-syllable out of thirty is what the display itself looks like at any moment.

**A picture is not the file it is a picture of.** `What a committed file may contain of somebody
else's work` governs what the tree carries, where a `.kar` in it *is* the song and plays; nothing
playable leaves with a PNG. The script regenerates from any corpus, so the pictures follow whatever
the demo corpus holds.

The corpus stays out of the repository as `CLAUDE.local.md` requires — the pictures are built from a
copy outside the tree — and `tools/dev/screenshots.sh` **will not write a published picture on a machine
that cannot reach a corpus**, so nobody can replace one with a fixture by accident.

**Two things in the pictures are fabricated rather than photographed, both because the output is
public**: the addresses in the connect panel, because the real panel prints a real LAN address and the
QR code encodes it; and the queue, which is a party rather than whatever happened to be queued when the
script ran. Anything else that leaks a machine is a bug in the script, not a thing to crop afterwards —
and the folder path in km-package-builder's header is the case that proves it: the folder the builder is
pointed at is chosen by `tools/dev/screenshots.sh` and is deliberately not the operator's scratch tree,
so the path in frame names a standard shared location and nobody's machine.

**Two rules about which songs may be photographed, both about the reader rather than the owner.**

*Every song a picture shows is in English*, because the README is: a television full of words most of
its readers cannot read demonstrates the wipe and nothing else. The catalog behind them is **not**
English-only, deliberately — `km-remote-pages` draws its language picker only where there is more than
one language to pick between, so an English-only corpus would quietly delete a control from two
published pictures and make the README's own alt text false. Keeping songs *in frame* English is
therefore done by the capture rather than by the corpus: the two list pictures are taken with
`?language=en`, which is deterministic where alphabetical order is luck, and which incidentally shows
the language filter in use instead of sitting unused on `All`.

*And not the songs everybody has already heard* — Hotel California, Yesterday, Let It Be. This one is
**curation and not code**: nothing filters by fame, and there is nothing it could consult if it did. The
argument is the same one that rejects `Song 1001 / Even Artist`, carried one step further: a catalog of
the five most-covered songs on earth also reads as a demo rather than as somebody's collection.

## What a published picture says about the build it was taken from

**Nothing.** The idle screen draws the build number in its bottom-left corner; the published picture
of that screen does not, and `crates/playback/km-display/examples/screenshots.rs` passes
`version: None` for the idle frame as well as the playing one to say so.

The README is read by somebody deciding whether to try this. A number in frame invites that reader to
compare it against the current release and read the project as behind, which is a fact about when a
screenshot run happened rather than anything about the product. The pictures are refreshed at a
release or when a screen changes, so a number in one of them is wrong more often than it is right.

**This is not the debugging-control rule one step further.** The bank label, the SoundFont label and
the performance overlay are kept out because they are drawn for a developer, and a README picture
carrying one advertises a machine nobody is shipped. The build number is drawn for a person, in every
build, and is left out for what it does to a reader instead.

## Which wallpaper the published pictures are taken over

**The one a fresh install shows: the first image of the shipped pack, picked by the same name ordering
`Playlist` gives the running machine.** The shipped pack is CC0, and it is the one a reader will
actually see, so it is the honest thing to photograph. A README picture of a background the product
does not ship is worth less than the megabyte it costs.

**Picked by position, not by name, and specifically out of the pack.** The pack's filenames carry a
content hash (`scenery-001-17193f9f-1920x1080.jpg`), so naming one would break at the next repack; and
`assets/wallpapers/` is a folder anything may be dropped into, where a stray PNG sorting ahead of the
zip would quietly put something else in frame. The example walks the playlist to the first entry
belonging to `default-wallpapers.zip` and decodes it through `Loader` — the same scan, the same
ordering, the same `Fit::Cover` — so the picture is not a second opinion about what the display draws.

**The cost is real.** A smooth gradient is the easiest thing a PNG ever compresses; a starfield is
not. `screen-playing.png` is about 1105 KB against a gradient's 460, and the eight pictures together
2.9 MB against 1.3, so the per-file and total budgets in `tools/dev/screenshots.sh` are 1 MB and 3
MB rather than 600 KB and 2 MB — a budget nobody can meet stops being a budget. Both are advice and
neither fails a run, which is why raising them is a comment change and not a loosened check.

**The gradients are kept, in `examples/wallpapers.rs`**: they are the one set that cannot fail the
contrast gate, because they are drawn against it, and they are what to reach for the day a
photograph has to be withdrawn.

## Who the README is for

**The README is for somebody who has the machine; `BUILDING.md` for somebody who has the source.** The
split is by **audience** and not by size, and the test for a paragraph is whether an installed build
can act on it. The cargo alias table, the Taskfile, the per-platform prerequisites, ffmpeg and
libclang, the ten staging scripts, what an IDE does with `ffmpeg-sys-next` and what GitHub bills for a
macOS CI minute are all `BUILDING.md`'s. The README covers installing, using, and the two products an
owner runs beside the machine — the offline remote and `km-package-builder`.

**Within the README the owner comes first, and `For a technical reader` is where the second audience
starts.** What it is, what it looks like, installing and using it are answerable from the machine's
screen, a phone, a browser or a double-click; the flags, the HTTP API, discovery on the network, what
a song file may contain and the commands that build a package need a shell or a program of your own,
and those sit after the divider. A bullet holding both is split across it: an owner is told a song
can be a video, and the codec it carries is stated below. Both halves still pass the test above: an
installed build can act on either.

**Its headings are addresses.** `#installing`, `#the-song-book` and `#getting-a-corpus-into-shape`
are linked from `BUILDING.md`, `DEPLOYING.md` and `Distribution`, and the two halves link to each
other by anchor as well. Nothing in the repository validates a markdown anchor, so a heading that
moves or is renamed takes its inbound links with it by hand.

**`BUILDING.md` sits at the root** rather than under `docs/` for one mechanical reason and one social
one: every relative link in it (`docs/decisions/*`, `docs/images/*`, `tools/*`) resolves unchanged,
and a file beside the README is found by somebody who did not think to look in `docs/`.

**Installing names the carriers and links one download**: the release page of the repository the README
is rendered from, which GitHub shows to whoever can see that repository. A link per asset would put a
version number into a document that carries none — every asset name has one in it — and each such link
would break at the next release. Building from source stays in the section as the other way in,
pointing at `BUILDING.md`.

**What is *not* duplicated**: the README says the builds are unsigned and what the recipient clicks,
`BUILDING.md` says how to sign one; the README states the logging behavior, `BUILDING.md` states why
`--frame-stats` is deliberately not a log level. It is reference documentation and not a plan, so
rule 5 in `CLAUDE.md` does not reach it.

## How a document in this repository is written

**A document states the rule and the reason somebody would need in order not to undo it. It does not
narrate how the rule was arrived at.**

What that excludes, in order of how often it creeps back:

- **What something used to be.** No former names, former defaults, former behaviors, no "this reverses",
  "this used to say", "since renamed", "no longer". A reader arrives at the repository as it is; a
  sentence about a state that is gone costs them a paragraph and tells them nothing they can act on.
  This covers a decision that was reversed as much as a spelling that changed.
- **Chronology.** No milestone numbers, no dates in headings, no "for two milestones", no ordering of
  when things were found. A date belongs in the body only where a reader needs to know when a
  measurement was taken.
- **Meta-commentary on the writing.** Any sentence whose subject is the document:
  "recorded rather than glossed", "and that is the record of it", "this row used to",
  "worth saying out loud".
- **Reassurance and common sense.** A paragraph explaining that a first run works, or that a diagnostic
  is optional, is a paragraph nobody needed.
- **Appositive tails.** "…, which is what makes X safe", "…, and that is deliberate rather than an
  accident" — the clause after the comma usually restates the clause before it.

What survives is the imperative and the trap. **A heading that instructs is not verbose** —
`Buffering the bank read makes it slower — do not try it again` *is* the content. A heading that merely
describes is trimmed to a plain noun phrase.

**The keep-test for a paragraph**: would a reader who deleted it either re-derive a wrong answer, or
break something silently? Anything in the past tense about a decision that no longer holds fails it.
When unsure, keep the sentence and delete the paragraph around it.

**This applies to code comments too**, on the same test. A comment saying why a line is the way it is
earns its place; one describing a state that is gone does not.

**A sentence takes the shape ASD-STE100 gives it.** The standard is Simplified Technical English, and
aerospace maintenance manuals are its home. The half above says what a sentence may be about. This
half says what shape it takes. Both bind every word this repository holds.

- **The active voice.** The sentence names the thing that acts.
- **One idea in a sentence.** Split a sentence before it carries two.
- **Twenty-five words, and the limit is hard.** A longer sentence becomes two. The standard sets
  twenty for a procedure and twenty-five for a description, and what this repository writes
  describes.
- **Simple tenses.** Present, past and future.
- **One word for one idea.** Never a synonym for variety, which
  [`One spelling per concept, across every surface`](foundations.md#one-spelling-per-concept-across-every-surface)
  already asks of a name.
- **No idiom, no slang, no metaphor**, in body prose.
- **The articles stay.** Write *the function* and *a race*.
- **Three words in a noun string.** A preposition breaks a longer one.
- **Six sentences in a paragraph.**

**An exact item keeps its spelling.** A path, a flag, a config key and an environment variable are
exact. So are an error string, a number, a version and an id. A plainer version of a name is a wrong
name. A concept word goes the other way: write *use*, never *leverage*.

**A longer document becomes more short sentences.** It never becomes fewer facts. The keep-test above
decides what a document says. The word limit decides only where the full stops fall.

**A full stop is not the only join, and a run of fragments is the failure to watch for.** Two clauses
that carry one idea keep their conjunction, their colon or their semicolon. A clause with no verb is
a fragment rather than a sentence, and three in a row read like a telegram. Where a split leaves
that, join the pieces back and spend the words the limit allows.

**A heading keeps the voice it has.** The rule above defends a heading that instructs, and this
machine's vocabulary lives in those headings. No word limit and no one-idea test reaches a heading.
The same holds for a table cell, a fenced block, a quoted block and a block of HTML. A picture's
`alt` describes one image in one breath, so it stays outside too.

**A commit message states the fault the change answers and the rule that holds after it.** The
subject is that rule in the declarative the log is written in. The body is the fault as somebody
standing in front of the tree would meet it, and the reason this change is the answer to it. A fault
a fix removes is stated in the past tense and *is* the content, exactly as a `Fixed` entry in the
changelog is — the past tense that fails is the session's own: the attempts, the order things were
found in, a count before against a count after, a commit or a message referred to as a thing. The
keep-test is the one above, asked of somebody reading `git log` a year later rather than of the
person who wrote it.

**It binds the next commit, and a message already written is left where it is.** Rewording one
changes every hash below it, which costs every reader more than the sentence saves. A branch's own
unmerged commits are where a reword is free, and they are the range the checker reads.

**And to every word this project publishes**, which is the reach that has to be stated because it
leaves the tree: the release page on GitHub, the tag message, the changelog entry, the site, and the
words inside a program. A reader who arrives at a release page arrives at the product rather than at
its history, so a sentence there about a state that is gone costs them the same paragraph it costs
anybody else -- and a release page is read by more people than every document here put together.

**Nothing published is typed at the point of publication.** The release page's body is
[`tools/dist/release-notes.md`](../../tools/dist/release-notes.md) and the site's is `site/`, both
tracked, so `check-prose.sh` and `check-no-local-refs.sh` read them through `git ls-files` like any
other file; `tools/dist/release.sh` substitutes the version. A body pasted into the GitHub form is
read by neither checker, which is the whole reason the file exists. The tag message is the one line
with no file behind it, and `karaokemachine X.Y.0` is all it says.

**`tools/dev/check-prose.sh` keeps the mechanical half true.** It matches the three shapes above that
have one -- `what something used to be`, `chronology`, `meta-commentary` -- and cannot see the
appositive tail or the paragraph of reassurance, so a clean run is a floor rather than a pass.

**`tools/dev/prose-sentences.awk` reads the sentence shapes.** It counts the words in a sentence and
the sentences in a paragraph. It catches a passive verb that names its agent. Idiom, metaphor and a
long noun string have no shape, so a person still reads new prose. It reads `*.md` and commit
messages, and it hands a code comment to that person.

**A page is prose only where the list names it.** `-v page=1` reads HTML instead of Markdown. A tag
becomes a space, and a `</p>`, a `</li>` or a heading's close ends a paragraph. It never reads a
`<script>`, a `<style>`, a comment or a heading's own words. The tracked pages are mostly Fluent
templates, where a line is markup and a sentence counter would report the markup. Naming the page in
`prose-converted.txt` is what makes it prose, and the site's two pages are what that is for.

**Structure is what reaches a page in another language.** The active voice, one idea to a sentence,
the word limit and the paragraph limit hold in any language. The vocabulary half does not travel,
because the checker cannot read Portuguese idiom. Neither the standard's approved word list nor
[`The prose and the names are US English`](foundations.md#the-prose-and-the-names-are-us-english)
reaches a translation, so a person reads a translated page for the rest.

**The tree converts one document at a time.** `tools/dev/prose-converted.txt` names the documents
somebody has already written to the shape, and every mode reads those whole. Outside that list the
checker reads only the lines a branch adds. More than a third of the sentences in this tree run past
twenty-five words. A tree-wide sentence run would therefore fail every branch, and a gate that always
fails teaches a session to skip the gate.

**`--commits` puts the same shapes over the messages, and its reach is `origin/master..HEAD`.** The
subject and the body both, merges included, because a merge here carries a written subject rather
than git's default. A hit names the short sha and whether it fell in the subject or the body, so the
reword is `git commit --amend` or `git rebase -i --reword` on a branch nothing has been built on yet.
**No message on the default branch is ever read**, which is what makes the rule enforceable without a
rewrite of history behind it.

**`task check` runs it over the lines a branch adds, and `ci.yml` reads the whole tree.** **Both,
and not one or the other** — a session can push without typing the task, and a runner cannot tell a
session that did from one that did not.

**On a pull request, CI reads the commit messages too.** That checkout carries the full history and
the pull request's own head rather than GitHub's merge commit, so `--commits` resolves the branch
against `origin/master` exactly as the task does. A push to `master` has no branch to read, and
`master` takes pull requests only.

**`--changed` in the gate and the whole tree by hand, and the reason is a cost.** Fourteen shapes
over 700 files is ~10,000 `grep` spawns -- 2m56s on this repository's Windows box, of which two
thirds is process creation, against 3.3 s for a branch's own lines. The sentence shapes are one
`awk` per converted document, which is about a second of that. Reading everything before every push
would put the slowest guard in the repository in front of a 13-second one. That is the opposite of
the order `task check` is arranged in. The whole-tree form is what to run after a large rewrite, and
what a runner should be given. It needs no history and no remote ref, where `--changed` resolves a
base against `origin/master`.

**A run that reads nothing fails.** Both modes had a path to reporting a clean tree having opened no
file at all: `--changed` fell back to diffing HEAD against itself where `origin/master` was absent,
and the whole-tree form printed the same where `git ls-files` could not run -- which is every run
inside the Debian image `task check:linux` uses, since it carries no `git`. Each says which of the
two is missing and exits non-zero. A checker that cannot read is worth less than no checker, because
it answers.

**Three files are exempt and say so in place.** `docs/learning-rust.md` is a teaching document,
where the arc from wrong to right *is* the content. `docs/HISTORY.md` is the origin story, where the
chronology and the abandoned attempts *are* the content; it is exempt from the rule and not from rule
5 in `CLAUDE.md`, because it states what was rather than what will be, and it binds nothing.

**`CODE_OF_CONDUCT.md` is the third, and it is somebody else's text.** It is the Contributor
Covenant word for word, and its last paragraph names the version it is. A sentence rewritten to the
shape would make that line false, so the file states the Covenant and this rule leaves it alone.

## Where a folder-scoped instruction lives

**An instruction that only matters once you are inside a folder lives in that folder's own
`CLAUDE.md`. An instruction that must fire before the first command stays in the root one.**

The root `CLAUDE.md` and every parent above it are loaded at launch. A `CLAUDE.md` in a subdirectory
is discovered but held back — it enters context when a file in that subtree is read. So the root file
is charged to every session and a nested one only to the sessions that go there.

The split is a test, not a size target:

- **Nested**, because it is wrong-able only by somebody already editing there: the second cargo
  workspace under `tools/cmd/assets/`, what `km-pack build` takes, why `fixtures/` holds no songs.
- **Root**, because it is wrong-able before any file in the relevant tree is opened: `--all-features`,
  the `target/` directory, the non-loopback bind, `--api-bind` with `--data-dir`, the worktree rule,
  the non-goals, and what a committed file may say about this machine. A lazily-loaded copy of any of
  these arrives after the damage.

**`@path` imports are not a size remedy.** They are expanded at launch alongside the file that names
them, so splitting a memory file across imports changes where the text lives and not what it costs.
Plain markdown links — what the table at the top of `CLAUDE.md` uses — cost nothing until something
reads them, and are the mechanism for anything long.

**A nested `CLAUDE.md` is tracked, so a worktree carries it** with no help from anything.

**`CLAUDE.local.md` splits the same way and needs that help.** The machine-local notes obey the same
test — this box's Docker and appliance notes in `tools/platform/linux/`, its ffmpeg paths in
`crates/playback/km-video/`, the ports and the shell's missing `PATH` at the root, where every
session needs them. Two things follow, and neither is optional:

- **`tools/dev/worktree.sh` finds them rather than listing them.** git carries none of these, so the
  script copies each one in. A literal list is the copy that silently stops being complete when a
  seventh note is added to a folder nobody anticipated.
- **The root file keeps a table of where the others are.** A nested note loads only when a file in
  its subtree is read, and the session that needs the appliance box's ALSA notes is often in
  `km-audio` instead. The table is what stops the note arriving too late.

**The root file keeps no description that [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) or
[`BUILDING.md`](../../BUILDING.md) already carries**, and a folder-scoped instruction is moved rather
than copied. A third copy costs the same wherever it is kept.

**A folder with its own `README.md` is the same rule one level down.** The nested `CLAUDE.md` beside
one carries the imperative that must not be missed — `icon/` is generated, `fixtures/` holds no
songs, `site/` is previewed through the script — and links to the README for everything else. It is
not a second copy of it.

## A worktree lives inside the checkout

**A worktree of this repository lives at `.claude/worktrees/<name>`, and both routes to one put it
there** — Claude Code's `EnterWorktree` through the `WorktreeCreate` hook, and
`tools/dev/worktree.sh` typed by hand.

**The location is what decides whether entering one costs a confirmation.** A Claude Code session
holds a permission root, and a worktree inside it is entered silently where a worktree outside it
raises a prompt the session cannot answer for itself. Nothing configures that away: the check runs
ahead of permission evaluation, so an `EnterWorktree` entry in `permissions.allow` does not reach it,
and neither does `additionalDirectories`, which grants access to files and not the right to move the
root. `.claude/worktrees/` is where the root already is. **This repository asks a session to enter a
worktree before it touches any tracked file**, so a prompt there is the one paid most often.

**The location charges two things, and the first is a search from the repository root**, which finds a
second complete copy of the source and reports every hit twice. Three things answer that, and between
them they cover what this repository actually runs:

- **`rg` honors `.gitignore`**, and `/.claude/worktrees/` is ignored, so every ripgrep-based search
  walks past it — including the search a Claude session itself runs.
- **Every check here reads through `git ls-files`**, which lists no ignored path.
  `check-no-local-refs.sh`, `check-prose.sh` and `check-mdns.sh` therefore cannot descend into a
  worktree whatever it holds.
- **What is left is `find` and plain `grep` from the root**, and that is the cost `target/` already
  imposes on both. A tool of this repository's own that walks the tree prunes the directory:
  `seed_local_notes` in `tools/dev/worktree.sh` does, and for it the prune is load-bearing rather than
  tidy, since without it a second worktree is seeded with the first one's `CLAUDE.local.md` files.

**An editor that indexes by directory rather than by `.gitignore` needs the folder marked excluded**,
which is a setting of the editor's and not of this repository — `.idea/` is not tracked.

**The second charge is a configuration file a tool reads from every parent directory, and it is paid
in the file rather than in the tree.** Cargo walks up from the directory it runs in, collects every
`.cargo/config.toml` above it and merges them: a string takes the nearest file's value, an array is
*joined* with the ancestor's. A worktree is a descendant of the checkout and is handed both copies, so
every array in that file arrives doubled and `cargo km-build` expands to `build … build …`. **Every
value in `.cargo/config.toml` is therefore a string**, aliases and `rustflags` alike, where the
nearest file wins and the two copies resolve to one; cargo splits a string on spaces, so the forms are
the same arguments, and no value there has a space inside a single argument.

**`tools/dev/check-cargo-config.sh` refuses an array, and the check is the point rather than the
convention.** The two forms look interchangeable — cargo's own contributors say so — and the one the
Cargo Book shows first is the array. Somebody adding an alias writes it, runs it where they are, sees
it work, and has broken every worktree in the repository; a comment is read by whoever was already
looking. `task lint:cargo` runs it beside the other text checks. **What the array form buys is an
argument containing a space**, which cargo cannot express in a string and which therefore belongs in
`Taskfile.yml`, read from one directory and never merged.

**A worktree and the checkout that disagree about the form are worse than a doubled array.** Cargo
refuses to merge an array with a string, fails to load its configuration at all, and every cargo
command in that worktree stops rather than one alias misbehaving. A worktree checked out before this
form reaches the default branch takes it before cargo works there again.

**Nothing else in the tree is found that way and joined.** `rust-toolchain.toml` and `rustfmt.toml`
are discovered by the same upward walk and taken whole from the nearest file, which in a worktree is
the checkout's own content; `Taskfile.yml` is read from the directory `task` runs in. What this leaves
to watch is the shape rather than the file: a configuration that a tool *joins* rather than replaces
cannot live in two copies of one checkout.

**`tools/dev/worktree.sh --where <name>` prints the path, and is the only place the rule is spelled.**
The hook asks it rather than computing the same path a second time, because a second copy is one that
agrees until it does not and then fails after a worktree has already been created. The answer is
resolved against the main checkout, so running the script from inside a worktree makes a second
worktree beside the first rather than one nested inside it, each level of which would carry another
`target/`.

**`--path` overrides it**, for a worktree that has to go somewhere else. Such a worktree is entered
with the prompt, and it is removed with plain `git worktree remove` rather than through the script.

## The license, and where its text lives

**`MIT OR Apache-2.0`, at your option, with both texts in the repository and in every carrier.** MIT
asks that "the above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software", so a folder handed to somebody carrying only the *words*
"MIT OR Apache-2.0" does not satisfy the license this workspace chose for itself. `LICENSE-MIT` and
`LICENSE-APACHE` sit at the root, and `dist_stage_app_licenses` in `tools/dist/common.sh` copies them
into every carrier — the Windows folder, the macOS bundle's `Contents/Resources` (before the seal, so
they are signed), all six tool folders, the tarball's `LICENSES/`, and the Windows installer, which
names them unconditionally beside ffmpeg's.

**The `.deb` is the one exception and is not an oversight**: Debian policy already requires
`/usr/share/doc/<pkg>/copyright` and has opinions about what goes in it.

**The pair is kept rather than narrowed to MIT alone**, which was considered: Apache-2.0 carries an
explicit patent grant that MIT does not, dual licensing is what the Rust ecosystem expects of a crate,
and — the load-bearing part — two decisions here rest on this being the license. `The tarball's ffmpeg
is built, not borrowed` and `Video in a macOS release` both refuse to redistribute a GPL ffmpeg
*because Apache-2.0 is not GPL-2-compatible*. Relicensing to MIT alone would quietly remove the premise
from under both of them.

## A tracked file states what is true and argues no legal position

**A file here says what this project ships and what a source's own terms say. It does not weigh a
risk, rate somebody else's claim, or say what would follow from a choice** — those are conclusions
this repository is not the place to publish, and a reader can reach their own from the same facts.
Nor does any file tell a reader that a consequence is theirs.

**The rule is about the framing and not about the subject.** Licensing is discussed here constantly
and has to be: `The tarball's ffmpeg is built, not borrowed` turns on which licenses can be combined
in one artifact, `Where a wallpaper pack's photographs may come from` names the three a shipped pack
accepts, and the bank table prints sixty-odd sets of terms. All of that is a statement of what the
project does. What the rule excludes is the layer above it: a position argued on top of the rule
being recorded, a characterization of somebody else's legal status, an editorial suffix on a row
that already prints its own terms.

**Attribution text is not covered by this and must never be trimmed to satisfy it.** `LICENSE-MIT`,
`LICENSE-APACHE`, `assets/soundfont/LICENSE.txt`, `assets/wallpapers/CREDITS.md`, both
`htmx-LICENSE.txt` copies and the LGPL terms `dist_stage_ffmpeg` puts beside ffmpeg are obligations
that carriers ship *because* the licenses require it. Removing one breaks a license rather than
tidying a sentence. See `The license, and where its text lives`.

**The gates in code stay too**: `km-carols`'s status check and `km-wallpaper-pack`'s allow-list are
what keep material out of a pack that would otherwise reach one. This row governs prose, not
enforcement.

## What a committed file may say about the machine it was written on

**Nothing.** No tracked file names a local drive or folder, a home LAN address, personal hardware, or a
person — anywhere in a tracked file, including inside a published PNG. **The repository is the owner's;
the documents in it are the product's.** A drive letter in one is noise to every reader who does not
have that drive, and a small leak besides.

**Where the detail goes is `CLAUDE.local.md`**, which is untracked and exists for exactly this.

**Nor how large the corpus is.** A file count, a song count, a row count, a database size, or a
spelled-out figure standing in for one states how much music the owner holds, which is a fact about
a person rather than about the product. A tracked file says *large*, *a whole corpus*, *hundreds of
thousands*, or the proportion the measurement actually turns on: a percentage carries the argument
where a denominator carries only the collection. **A sample stays a number** — *4,000 `.kar` files
sampled from the local corpus*, *measured over 3,795 corpus files* — because it names the work done
and says nothing about the size of what it was drawn from. `check-no-local-refs.sh` cannot see any
of this: six digits have no shape separating a corpus from a byte count, so it is held by the same
human pass as hardware and people.

**A document that has to stay reproducible names a variable, not a path** — `tools/dev/soundfont-measure.sh`
lists the seven songs a soundfont comparison is measured on, and reaches them through `$KM_CORPUS`
with the root recorded locally, which keeps the reproducibility and drops the machine.

**Five things are deliberately excluded.**

*Published identity* stays: the license copyright, the **author line** in `README.md` and in the two
site footers, the `.deb` `maintainer` field, the installer's `AppPublisher`, the repository URL,
the bundle id `com.rrgmc.karaokemachine`, the **Apple team identifier** in
`ports/remote/ios/project.yml`, and the two **Developer ID certificate common names** in
`tools/platform/macos/installer.sh` — these are how the product is signed and addressed, not
where it was built. The team identifier is the one that has to be argued rather than assumed, because it
is ten characters that look like a secret and are not: it is public in every signed build Apple
distributes, and Xcode refuses to sign without it while XcodeGen writes an empty one over any value left
at project level, so an unset placeholder is a project nobody can build. It is also a *second* place a
team identifier appears — `tools/dist/common.sh` derives one in `dist_team_id()` out of
`KM_SIGN_IDENTITY`, under an explicit "one variable read in one place" rule — and the two do not have to
agree: that one is a **Developer ID**, and is about distributing a macOS build
outside the App Store, where this is an **Apple Development** certificate under automatic signing for
putting a build on the owner's own phone. Same team, two signing worlds. If they ever must be one value,
the fix is `tools/port/remote/ios/build.sh` preferring `dist_team_id` when `KM_SIGN_IDENTITY` is set and
substituting it into the generated project — not a third literal.

**The certificate common names are a separate argument from the team identifier, and need one**, because
where that is ten anonymous characters, `Developer ID Application: <name> (<team>)` carries a person's
name — the thing this decision otherwise forbids outright. It stays on the same test the rest of the
list passes: a common name is printed by `pkgutil --check-signature` from any package signed with it and
by `codesign -dvvv` from every bundle inside, so it is **published by the artifact** rather than
describing the machine that built it. It is also already here in another form, the license copyright
naming the same person for the same reason. What is bought is that `task dist:setup:notarized` and
`tools/platform/macos/installer.sh --notarize` produce a shippable installer with nothing set up first;
an environment variable still overrides, so nobody else has to edit a tracked file to sign as themselves.

**They are defaults on the `--notarize` path only, and that gate is what keeps the rest of this true.**
`dist_signing` is `[ -n "$KM_SIGN_IDENTITY" ]`, so a default applied unconditionally would make every
build sign — which `tools/dist/common.sh` refuses on the grounds that a fresh clone, somebody else's Mac
and the CI runners have no certificates and must still be able to stage a build. `task dist:setup` is
therefore still ad-hoc, and *"ad-hoc is the default"* in the `Signing a macOS release` decision still
holds everywhere except the one path that asks for the opposite by name.

`tools/dev/check-no-local-refs.sh` needs no change for any of this, and there is nowhere in it to put
one: it matches three shapes — a drive-letter path, a private-range address, a home directory carrying a
name — and neither a team identifier nor a certificate common name is any of them. This exemption is
held by the human pass, exactly as the note below says.

*Generic OS paths* stay (`C:\Windows\Fonts`, `%LOCALAPPDATA%`, `/var/lib/karaoke`): they name the
platform, not a person.

*Second-person phrasing* stays — "run `tools/setup/fetch-ffmpeg.sh` once on this machine" addresses
whoever is reading. What is excluded is the deictic use meaning *the author's* box.

*Stock hardware strings* stay in `km-audio`'s ALSA fixtures: `USB Audio CODEC` and `HDA Intel PCH` name
a class shared by millions of machines, five separate test mechanisms depend on their exact bytes, and
realistic output is the whole value of a `/proc/asound/cards` parser fixture. What is excluded is a
caption saying whose appliance it was captured from, along with a real bus address and IRQ the parser
discards anyway.

*Named test devices* stay on the same argument, and eight tracked sites depend on it: "deployed to a
**Galaxy S23** (arm64) and a **Google TV Streamer** (armv7)" in `docs/architecture/android.md` and the
two Android READMEs, and the Samsung background-killer notes beside them. A retail phone sold by the
million names a class exactly as `USB Audio CODEC` does, a measurement is worth less when you cannot
tell what it ran on, and "an arm64 phone" would cost the reader the one thing the sentence is for. What
stays out is a serial, a MAC address, a name attached to the device, or a claim about the room it is in.

**The rule these five exemptions qualify is only half enforceable**: hardware and people have no
shape, so the check below cannot see them and a human pass over new prose is the only thing that
can. `CONTRIBUTING.md` says so in place.

**`tools/dev/check-no-local-refs.sh` enforces the rest, by shape and never by value** — a deny-list
containing the real corpus path would publish the thing it exists to hide, so it matches drive-letter
paths outside a small allowlist, private-range addresses outside the documented example set, and home
directories carrying a name. It is the one file allowed to contain those shapes, it runs first in
`task check` because it is the cheapest failure to read, and first in CI's `guards` job.

## Every fixture in the tree is synthetic

**No real karaoke file is committed.** A real file that exposes a bug earns a *minimal synthetic*
fixture named for the case it covers, in `km_song::testing`, and the file itself stays wherever it was
found. This is the sibling of the row above — that one is about the machine a file was written on,
this one about the files themselves.

**A synthetic fixture is the better test as well as the smaller one**: a real file pins a hundred
incidental properties and documents none of them, so when it breaks nobody knows which one mattered.
A builder named for its case says in its name what it is for.

**Four synthetic builders each name what they cover**: `soft_karaoke_header_on_words_track` (the layout
real files use — the magic alone on one track and the `@L`/`@T` header on *Words* — carrying thirty-two
lines and a `Tema` guide on channel 5 against a monophonic bass on channel 1 that hits every one of the
same syllables, which is the case that makes singable range a gate rather than a bonus);
`soft_karaoke_producer_credit_first` (a studio in the *first* `@T`, the positional-convention failure
that puts a studio in the title of hundreds of songs); `melody_on_channel_fifteen`; and
`named_text_track_credit_in_the_name`.

**A second fixture list exists because of a constraint rather than a preference.** Four sweeps
across `km-song` and `km-suitability` require every entry in `FIXTURES` to parse, so the files that
must be *refused* cannot live beside them: `UNREADABLE_FIXTURES` holds those four, and each refusal
is its own assertion where a sweep could only count them — a folder is open-ended and a `const` is
not. The FTP-damage one is worth one sentence: a corrupted track is *tolerated*, and what actually
kills the file is that thirteen is `0x0d`, so a thirteen-track file has `MThd`'s own count expanded
and every byte after it is one out.

**Encoding is tested at `TextDecoder::resolve` rather than through a file**, with genuine Japanese in
Shift-JIS and a Czech pangram in CP1250. A crafted misdetection would pin `chardetng`'s current
tie-breaks rather than any behavior of ours: a magic byte string found by search, failing a version bump
with exactly one legal remedy, which is to change the test. The claim it would illustrate — that the
manifest's `lyric_encoding` override has to exist — is tested directly.

**What does not follow**: a synthetic corpus does not become sufficient by this row. The standing
risk stands — lyric-format surprises come from real files — and what answers it is the owner's own
corpus through `km-lyrics scan` and `km-package-builder` over tens of thousands of files.

## A downloadable song pack

**This project publishes songs, and they are never bundled with the machine.** Sixteen Christmas
carols, one `.kmpkg`, downloaded by somebody who wants it and dropped into their packages folder.

**Carols, and not a general catalog, because of what a karaoke MIDI is.** It is not one work but four
or five — the composition, the setting, the words, any translation, and the sequence somebody typed in
— and each holds a status of its own. Hymnals are the bodies that have done that work per item and
written the answer down, so they are where a pack can be assembled from a stated status rather than
from an assumption.

**The Open Hymnal Project is the source**: its ABC Plus states a machine-readable status per tune and
aligns its lyrics to the notes, which is exactly what `km-carols` needs and what nothing else offers.
Everything it holds is devotional, so a karaoke machine whose *built-in* catalog were sixteen hymns
would misrepresent the product — while a Christmas carol pack somebody chooses to fetch in December is
exactly what it says it is.

**`assets/` holds none of it and no carrier changes** — no `.deb` whitelist line, no Android size
budget, no first-run install path, and `.gitignore`'s unanchored `*.kmpkg` needs no exception,
because the repository holds the recipe (`tools/cmd/km-carols`, `tools/dist/carols.sh`) and the
release holds the bytes, exactly as `dist/` does for every other carrier. A fresh install starts
with an empty catalog **on purpose**: `Every fixture in the tree is synthetic` is confirmed by this
rather than stretched, since the repository holds the recipe and never the songs.

**The gate on a tune's stated status is a constant in the code and fails closed**, following the two
rules `Where a wallpaper pack's photographs may come from` established for the same question about
photographs — and the first edition it is pointed at has a case for it: one carol's words and music
are public domain and its *setting* is CPDL's, so it is not in the pack.

**The pack is built by `km-pack build` from a description `km-carols` writes**, never by a second path
of its own, which is the rule `km-package-builder` already follows: two tools writing subtly different
manifests from the same songs is a defect nobody notices until a package behaves oddly. `abc2midi` is a
build-time dependency on the same footing as `ffmpeg` the command and Inno Setup — nothing links it,
nothing ships it, and its GPL therefore reaches no released artifact.

## The website is one page per language, and it links one download

**One hand-written `index.html` per language and one stylesheet in `site/`, in the machine's own theme
colors, deployed by Actions to `rrgmc.github.io/karaokemachine`.**

**Every screen this product has speaks the reader's language, and the page describing it does too.**
English is served at the root and every other language one segment down under its own tag —
`site/pt-BR/index.html`, the same tag `i18n/pt-BR.ftl` carries and the same one `Locale::tag` returns.
A page is whole rather than a body filled from a catalog, because a Fluent catalog holds no markup and
this page's sentences are inseparable from their own emphasis: splitting one around its `<b>` forces
English word order onto every language that follows. `Translations are governed by their own language`
in [`foundations.md`](foundations.md#what-a-user-reads-is-written-in-plain-application-language)
governs the words, so a page reads as though written in its language rather than rendered into it.

**The language is offered, never chosen for the reader.** A static host negotiates no
`Accept-Language` and the page runs no script, so each page carries a link naming the other language
in that language — `Locale::endonym`'s rule, on a page with no Rust behind it to read an endonym from.
`<link rel="alternate" hreflang>` is what tells a search engine which page to serve whom, and
`x-default` names English, which is also what the root serves.

**`tools/dist/site.sh` refuses drift between the pages**: the same sections, the same pictures, the
same links out, and each page declaring its own `lang` and reaching every other. That is the drift a
grep can see, and it is what stands in for the three catalog parity tests
[`Catalogs live beside the words they translate`](#catalogs-live-beside-the-words-they-translate)
owes from every translated crate. A paragraph that fell behind in *words* has no shape, and is found
by reading it.

**The links out stay English.** The repository, `BUILDING.md`, the architecture notes, the decisions
and the release page are English wherever the reader came from, and the Download button goes on
naming the repository the page deploys from.

**The footer names the author, and the address is text rather than a link.** `README.md` carries the
same name and the same address, so a reader meets one form in both places. A `mailto:` would ask the
visitor's mail client to open, which the page asks of nothing else. Each page translates the label
alone, and carries the name and the address verbatim.

**Not a generated documentation site**: the documents are some twelve thousand lines of markdown that
GitHub already renders with anchors, a file tree and search, and a generator would buy a second
rendering of the same words that can go stale against the first — the failure rule 5 in `CLAUDE.md`
exists to prevent, in HTML rather than in prose. What GitHub renders badly is a *first impression*, and
that is the one page this adds.

**No Jekyll, no mdBook, no MkDocs and no Node**, because the whole page is smaller than any of those
toolchains' lockfiles: the cost of a generator is never the generator, it is a second language to learn
before you can change a heading and a build that breaks for reasons unrelated to the page.
`crates/remote/km-remote-pages/static/app.css` makes this argument for the remote first and is the
working precedent.

**No external request of any kind** — no web font, no CDN, no analytics — which is `Nothing downloads`
applied to the page that describes the product: a machine whose whole claim is that it works on a
network with no internet on it should not have a home page that fetches a stylesheet from somebody else.
`tools/dist/site.sh` refuses one rather than trusting the promise, and refuses an absolute path in the
same pass, because the site is served one path segment down and `/images/x.png` would resolve to the
organization's root — the problem a Jekyll `baseurl` normally solves, solved here by not having it.

**The pictures are staged and never committed twice.** They are in `docs/images/` because the README
shows them and `tools/dev/screenshots.sh` regenerates them; a second copy under `site/` would be a
megabyte that silently diverges from what a reader sees on GitHub. The consequence is accepted rather
than worked around — `site/index.html` opened straight out of a checkout shows no pictures, and the
script is the preview. The alternative, a `../docs/images/` path that resolves in both places, was
rejected because it escapes the artifact root: it would 404 in production *only*, the local tree having
a real `docs/` above it, which is the class of fault `Nothing may assume cargo builds into target/`
already costs this repository thirty lines.

**One Download button, and it names the release page rather than a file.** An asset's name carries the
version, so a link to one asset stops resolving at the next release, where the release page holds every
carrier at one address that does not move. The carriers table says what each package is and links none
of them individually, which is `Who the README is for` carried onto the web.

**The button names the repository the page deploys from.** GitHub serves a release to whoever can see
the repository holding it and a 404 to everybody else, so a download URL pointing outside that
repository is a button that answers a visitor with nothing. `pages.yml` tests the deploying
repository's name *and* its visibility for that reason rather than the name alone: the page reaches the
web only from a public repository, which is the same repository that then serves the release behind the
button. Every URL in the page says `rrgmc`, the download among them.

## The Rust toolchain is pinned exactly

**`rust-toolchain.toml` names one `x.y.z` version, and it is the only place the number is decided.**

**The concrete cost of a floating channel is not reproducibility in the abstract** — it is that
`cargo km-lint` is clippy with `-D warnings` over a workspace that also warns `missing_docs`, so a lint
introduced upstream on a Tuesday fails CI on a branch that changed nothing relevant, and "passes
locally" means only "passes on whatever this machine last fetched". A pin moves both into a commit that
runs `task check` first. This is the ordinary convention for an *application*; a library would do the
opposite and commit no `rust-toolchain.toml` at all, so as not to impose a compiler on contributors or on
downstream testing. Everything here is `publish = false`.

**`rust-version` is kept equal to the pin.** Understating it is not merely untidy: there is no
`clippy.toml`, so `rust-version` *is* clippy's MSRV and a low one suppresses the lints that suggest
modern idioms, and `resolver = "3"` is MSRV-aware, so it also makes `cargo update` prefer needlessly old
dependencies. For an application the honest floor is the compiler it is built with; nothing downstream
reads this to decide anything.

**The number propagates rather than being repeated, which is `tools/setup/features.sh` reasoning applied
to a compiler.** The three workflows install with `rustup toolchain install --no-self-update`, which
resolves the file, components and all — `dtolnay/rust-toolchain` is deliberately not used, because its
`toolchain` input is required and it cannot read `rust-toolchain.toml`, so keeping it would mean the
version written twice. The Debian image takes it as a `RUST_VERSION` build argument that
`tools/platform/linux/image-tag.sh` derives, **and hashes into the image tag** — without that last part
a bump would leave the tag identical while the image it named still had the old compiler baked in, which
is exactly the silently-wrong-image failure that file exists to remove. Three files must still repeat
the number because no format lets them derive it — two `rust-version` keys and one sentence of
`docs/learning-rust.md` — and `tools/dev/check-toolchain-pin.sh` fails naming any that disagree, in both
directions: it also refuses a floating channel and a reappearing `dtolnay` step.

**The cost is accepted rather than automated away.** A bump is a commit every six weeks or so, and
`rustup target add` is per toolchain — so a bump silently drops the Android and iOS standard libraries
and the first port build afterwards fails its own target check. Both are written down in
`Bumping the Rust toolchain` in `BUILDING.md`. **Dependabot's `rust-toolchain` ecosystem would open that
PR the day each release lands and is deliberately not enabled**: a bump also wants the Android and
iOS targets added and the ports built, which CI does not do, so the PR would be green and incomplete.
`dependabot.yml` covers the workflows' actions only.

## Why the pass checks everything

**`task check` compiles and tests the whole workspace on every run, and it is kept quick by making its
stages cheap rather than by running fewer of them.** It is the gate before a push, and CI is the
second one, after it; a stage skipped here is found a push later, on a runner.

**Selection by changed crate is the one thing this command may not do.** The set has to be the changed
crates *plus their reverse dependency closure*, and the closure is widest exactly where work is most
common: `km-song` is in the closure of 31 of the 35 members, so the change most in need of skipping
something skips nothing. At the other end the prize is small — a touched leaf costs 12s of compilation
inside a 43s pass whose floor is the tests that run whatever changed. A false green on the first gate is
worth more than 12s.

**The saving that exists is in what a stage does per item, not in how many items it visits.** A
password hash compiled at `opt-level = 2` and an allowlist matched by the shell rather than by a
subprocess per hit are worth 26s of a warm pass between them and drop no check at all. Each is argued
where it lives, in the profile stanza of `Cargo.toml` and in the loop in
`tools/dev/check-no-local-refs.sh`.
The stage-cost table in [`BUILDING.md`](../../BUILDING.md) carries the figures.

**Clippy keeps `--all-targets` although the test build repeats its work.** The two share no artifacts,
so whichever runs second pays a full traversal, and that is 73s of the cold pass. What dropping it buys
is a workspace whose test targets are linted by nothing, on a repository where `-D warnings` over them
is the reason to run the gate rather than trust the build.

**One baseline, and no quick variant beside it.** A second, narrower task is a second answer to "did it
pass", and the narrow one is the one that gets typed.

**The excluded workspace is inside the gate, at the largest price the gate pays.** `tools/cmd/assets`
is a second workspace, so `--workspace` cannot reach it, and `km-admin` takes path dependencies on nine
crates under `crates/` — a change to `km-api` or `km-admin-pages` compiles green in the root workspace
and breaks there. `task lint` and `task test` therefore name both manifests, as `fmt` does. What that
costs is 13s of a warm pass and half of a cold one: the second workspace builds at `opt-level = 1` with
its dependencies at 2, and shares no artifacts with the root build.

## `master` takes pull requests, and CI is one required check

**Nothing is pushed to `master` directly: a change arrives as a pull request, and merges when the
check `CI ok` has passed.** That check is the last job of `.github/workflows/ci.yml` and needs every
other job there; it fails when one failed and passes when the rest passed or skipped.

**One workflow, whose jobs skip themselves, and no workflow with a path filter on its trigger.**
Branch protection names a check, and a workflow a path filter kept from starting never reports it, so
a pull request waiting on it never merges. A job whose `if` is false reports as skipped, and a skip
satisfies the requirement.

**What may skip is decided by exclusion.** The build jobs skip only when every changed file is
Markdown or under `docs/`, which nothing the build reads is; a new directory counts as code until it is
listed. The one inclusion, the second workspace's own job, can fail open after a move, and the weekly
scheduled run of everything is its backstop.

**All three desktop platforms run on every pull request**, because a public repository's standard
runners cost nothing, and a fault found on the pull request is cheaper than one found in a release.
**The video build runs on Linux, in `debian:13-slim`**, the base the appliance and the `.deb` are
built on, so it compiles against the ffmpeg they link. Android and iOS are not built on a pull
request; the release workflow builds them from a tag, as
[`CI builds the release, and a Mac adds its packages`](distribution.md#ci-builds-the-release-and-a-mac-adds-its-packages)
says.

## A vulnerability is reported privately, through GitHub

**A security report goes through GitHub's private vulnerability reporting, and nowhere else.**
[`SECURITY.md`](../../SECURITY.md) links the form and names no email address. A report arrives as a
draft advisory, which holds the discussion, a private fork for the fix and the advisory text in one
place, and publishes them together with the release. A mailbox would be a second place to watch and
would hold none of that. **The form is a repository setting**, so a fork or a move of the repository
has to turn it on again before `SECURITY.md` points at anything.

**Only the latest release is supported.** There is one version number for the whole repository and
no maintained branch below it, so a fix ships as the next release.

**No issue form asks for a song file.** Attaching one publishes it, and almost every song in the
world is somebody's copyright. The bug form asks for the file's shape instead, which is also what a
fixture here is built from. See
[`Every fixture in the tree is synthetic`](#every-fixture-in-the-tree-is-synthetic).

**Blank issues are off.** Every issue starts from the bug form or the feature form, and the feature
form points at the non-goals and `docs/decisions/` first: a request for something decided against is
a request to change that decision, and it names the entry.

## An issue carries the platform and the program it is about

**A label names the platform and the program, because that is what an issue list is asked.** The bug
form requires both, and an answer that reaches only the body cannot be filtered on: "what is broken
on Android" or "what is wrong with the remote" means opening every issue to find out. The labels are
`windows`, `macos`, `linux`, `android` and `ios`, and `machine`, `remote`, `package-builder`,
`admin`, `tools` and `api`.

**The form's answer is the source of the label**, and
[`.github/workflows/issue-labels.yml`](../../.github/workflows/issue-labels.yml) applies it when an
issue opens and when its body is edited. A person filling the form answers the question once, in the
place the question is asked; a label applied by hand is a second answer that agrees with the first
only as long as somebody keeps it agreeing. **An edit reconciles**, so an answer corrected to drop a
platform drops the label with it, and a facet the body does not answer at all is left alone, which is
what keeps a label somebody applied by hand.

**The optional kind-of-song answer stays unlabelled.** It is given only when the problem is with a
song, so four more labels sort a part of the list rather than the list.

**Every label is declared in [`tools/dev/labels.sh`](../../tools/dev/labels.sh), which is the only
place one is written down**, and `tools/dev/labels.sh sync` puts that table on GitHub. A label
created in the web interface is a label no checkout knows about and no guard can read, and the four
GitHub creates in every new repository that this table leaves out go the same way: `good first issue`
and `help wanted` offer work to a crowd that is not here, `question` cannot arrive while blank issues
are off, and `invalid` says what closing the issue says. `dependencies` is in the table although
nothing here applies it, because Dependabot applies it and creates it again when it is gone.

**The type labels keep GitHub's stock names.** `bug` and `enhancement` are what the two forms apply,
and a name every reader of a GitHub repository already knows is worth more than a shorter one.

**`task lint:labels` asserts that a dropdown option and a label still say the same thing**, both ways
round, and that an option carries no comma — a rendered multiple choice joins its answers with one,
so an option containing a comma cannot be told from two options. An option renamed in the form
without the table is the fault it catches, and the alternative is an issue that quietly arrives with
no label.

**The workflow reports no check.** The `issues` trigger never fires on a pull request, so it stands
outside [`master` takes pull requests, and CI is one required check](#master-takes-pull-requests-and-ci-is-one-required-check)
rather than against it, and it carries no path filter for the same reason that workflow carries none.
**The issue body reaches the script through the environment**, never through a command line: a body
is written by anybody, and interpolating one into a shell line is that person choosing what runs.

## A pull request carries its type, and the programs and platforms it touches

**A pull request carries the same labels an issue does.** The pull request list then answers "what
changed in the remote" or "what touched Android" without opening each one.

**The type label is chosen when the pull request is opened**: `bug`, `enhancement` or
`documentation`, passed to `gh pr create --label`. Whether a change fixes a fault or adds something
is a judgement, and no path answers it.

**The changed paths are the source of the program and platform labels.** A pull request has no form
to answer, and the folders it changes already name what it touches. The folder-to-label table is
`paths` in [`tools/dev/labels.sh`](../../tools/dev/labels.sh), beside the label table, so a label is
still written down in one file. [`tools/dev/pr-labels.sh`](../../tools/dev/pr-labels.sh) reads it,
and [`.github/workflows/pr-labels.yml`](../../.github/workflows/pr-labels.yml) applies the result
when a pull request opens and on every push to it.

**A folder that serves every program gives no label.** `crates/platform/`, `docs/`, `site/`,
`icon/`, `.github/` and the rest of `tools/` are in that group. A label that every pull request
carried would sort nothing.

**The labeller only adds.** A label put on by hand stays, and so does the type label. A push that
stops touching a folder leaves its label in place, because a pull request did touch it once.

**`task lint:labels` checks the path table too.** Each row names a declared label, and each folder
still holds a tracked file. A folder renamed without its row is the fault this catches. Without it,
every later pull request there quietly arrives with no label.

**The workflow runs on `pull_request_target`, and it runs nothing from the pull request.** That
trigger gives a pull request from a fork a token that can label it. The checkout takes the base
branch, so the script that runs is `master`'s. The pull request reaches it only as a list of paths.
It is not a required check and carries no path filter, so
[`master` takes pull requests, and CI is one required check](#master-takes-pull-requests-and-ci-is-one-required-check)
stands unchanged.

## One version number for the whole repository

**Every program here carries the machine's version, including the two in the excluded workspace.** A
version here is a folder name before it is a promise.

**The concrete cost of separate numbers is a script that cannot find what it has just built.** A staged
folder is `dist/<app>/<platform>/<app>-<version>-<triple>`; `tools/dist/cmd.sh` names it from the version
the *binary* reports and `tools/dist/bin.sh` looks for it using the version a *manifest* reports. If
those can differ per program, every script walking `dist/` needs an arm apiece — two chances to make the
same mistake per program added, and the result is a run that stages `km-admin-0.1.0-…` and then says
`nothing staged for km-admin` about the folder it wrote seconds earlier.

**The carriers assume it.** `tools/platform/windows/installer.iss` takes one `/DVersion=` and the
macOS `distribution.xml` puts one `@VERSION@` on all seven `pkg-ref`s — both read out of the
machine's binary.

**The number is written down twice and cannot be written down once.** `tools/cmd/assets` is `exclude`d
from the root workspace, and nothing — not a workspace key, not `[patch]`, not `.cargo/config.toml` — is
inherited across that boundary, which is the same constraint `rust-version` lives under one line away in
the same two manifests. So it takes the same remedy: `tools/dev/check-version-pin.sh` fails naming both
values when the two roots disagree, and also refuses a member that goes back to spelling its own version
out — because comparing the roots alone would pass while producing exactly the folder name this exists
to prevent. `task check` runs it beside the toolchain check.

**What this gives up is real.** The two programs cannot be released on a cadence of their own, and a
bump to the machine bumps them whether or not they changed. That is the right trade while they ship
inside the machine's own installers: they are not published, not depended on, and not separately
downloadable, and the day one of them is, this is one arm and one variable to put back.
`Bumping the version` in `BUILDING.md` is the two lines a tag means.

## A release keeps what it changed, and the changelog is where it is kept

**`CHANGELOG.md` holds a section per released version, and a release is not cut without one.** The
download page's body is a tracked file with a `What changed` list in it, and that list is rewritten
at every cut — so without somewhere for it to accumulate, each release's answer is destroyed by the
next one and the only record left is a tag message reading `karaokemachine X.Y.0`.

**It is written for the person the release page is written for**, in the same register and under the
same rule: a sentence per change, consequence first, naming what somebody with the machine gets. See
[`What a release page says, and to whom`](distribution.md#what-a-release-page-says-and-to-whom). The
reasoning behind any of it is a decision here and does not belong in an entry.

**Chronology is what this file is for, and the rule against it does not reach the headings.**
[`How a document in this repository is written`](#how-a-document-in-this-repository-is-written)
excludes dates in headings, and `what something used to be` with them, on the grounds that a reader
arrives at the repository as it is. A changelog is read by somebody arriving at a *version*, so the
date and the number are the content. The entries under a heading are written under the rest of the
rule, and `tools/dev/check-prose.sh` reads them as it reads any other file — which is why the file
needs no exemption where `docs/HISTORY.md` does.

**Its version numbers are records and never advance.** They are the one deliberate set of hits in the
`git grep -F <old version>` that follows a bump, where the eight places in
`Eight more places, and nothing checks them` are numbers that have to move. A substitution run through
this file rewrites the history of a release rather than the version of the next one.

**No compare links.** Keep a Changelog puts one under each version; twelve of them would be twelve
more absolute URLs naming the account this repository currently sits under, to be rewritten the day
it moves and dead in every clone until then. A version and a date locate a release in `git log`
without help.

**The order is a document of its own.** Retitling `Unreleased` is the first step of a cut rather than
a line inside `BUILDING.md`'s bump section, because the range it is written from is read most easily
before the tag exists — and a release has six more steps after it that no single section owned.
[`RELEASE.md`](../../RELEASE.md) is that order, and it points at `BUILDING.md` for every command
rather than repeating one.

## Catalogs live beside the words they translate

**`crates/platform/km-locale` holds the machinery and no product words.** What a locale is, how a
browser's `Accept-Language` becomes one of ours, what happens when a key is missing, the cookie a
chosen language is remembered in, and the askama filter — the part seven surfaces would otherwise
answer seven ways. It never depends on anything it is translating, which is what lets it sit below
every layer.

**The cookie is here whole — its name, its path and its life — rather than only its name.** Two
programs write it: the singer's remote, and `km-admin`'s front door on an origin of its own. A name
shared while the rest was spelled twice is a surface forgetting what another remembered, which is
the same failure the name was moved here to prevent.

**Every catalog is in the crate whose words they are**, `include_str!`-ed by it: `km-display/i18n/`,
`km-remote-pages/i18n/`, `km-admin-pages/i18n/`, `km-api/i18n/` for the printed book,
`tools/cmd/assets/km-admin/i18n/` and `tools/cmd/km-package-builder/i18n/`. That is the same judgement `km-admin-pages` already makes
by copying its own asset hash rather than sharing one — these words belong to *these* pages, and a
rename that orphans a key should be caught in one crate rather than found on somebody else's screen.
`include_str!` for `Bundling assets`' reason: this code ships inside a `.deb`, a macOS bundle, an APK
and an iOS app.

**A program whose pages are half somebody else's keeps a catalog for its own half only.** `km-admin`
draws *This machine*, Songs, Pictures and Sound from `km-admin-pages` and its front door, picture
searching and bank fetching from its own templates, so it renders its body from its own catalog and
asks that crate to draw the chrome from theirs, with one locale read from the request feeding both.
Its keys in the pages crate's catalog would sit beside markup that never spends them, and that
crate's own `no_message_is_left_unused` refuses exactly that.

**Outside `crates/` is not an exception**, because the rule is about what a catalog sits next to:
`tools/cmd/assets/km-admin/templates/` and `tools/cmd/km-package-builder/templates/` are markup like
any other.

**Under `platform/` and not a sixth folder.** Those five folders are the dependency layering written
down, and `platform/` is already what sits below all of it. A crate holding no karaoke concept is
exactly that shape, and a sixth folder for one crate would weaken the claim the five make.

**No cargo feature is added to the workspace's list.** `km-locale`'s `askama` feature is turned on by
the four manifests that render markup, which is the pattern `km-remote-core`'s `mdns` and `sweep`
already follow — `tools/setup/features.sh` is for features a full build has to be *told* to enable,
and one a consumer names for itself is already on wherever it matters.

**Three tests are owed by every crate with a catalog**, and they are what stand in for the
compile-time checking Fluent cannot give: every key in `en` is in every other locale, no locale has a
key `en` lacks, and every key the markup or the Rust asks for exists. The markup is scanned, crudely
and deliberately — a key in a template is a string literal askama passes through untouched, so a
rename that misses one compiles and renders `⟦tab-songs⟧` on somebody's phone.
