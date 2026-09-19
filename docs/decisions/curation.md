# Curation

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Curation tool

**A local web server serving HTMX pages — `tools/cmd/km-package-builder`.** The name says what it
produces, as every sibling tool does (`km-pack`, `km-lyrics`).

The work is browsing and sorting hundreds of thousands of rows, which a browser does well and an SDL
keypad screen does not. Every operation underneath already exists as a library, so a second native UI
would be new code for no new capability.

It is a **separate binary from the machine**, not a page inside it: it writes packages, opens files in
the OS, and indexes a corpus. None of that belongs in an appliance under a television. It binds
loopback and has no password of its own -- though it holds the *machine's*, for the run, once
somebody types it.

**Its page is dark, and pinned dark** — one palette, `color-scheme: dark`, no `prefers-color-scheme`
branch. A desktop in light mode gets the same page as one in dark. A light ground is unambiguously
better for a dense table of small text read for hours. But it lost to the owner's own preference on
sight. **Taste about one's own instrument settles this, and a general claim about legibility does
not.** The next surface to raise the question should be answered by looking at it, not by reasoning
about contrast.

The singer's remote is light for a reason of its own that does not apply here. The television is
dark because it is a television. Three surfaces, three separate decisions.

## The order of the tool's pages

**The navigation follows the arc of the work.**

- Songs is where curating happens.
- Lyrics is that same list reached through the words, which on a corpus of untitled files is the
  only thing many songs say about themselves.
- Favorites is where a decision about a song is put.
- Packages is what the decisions are for.

Find it, read it, file it, build it.

Folders and Duplicates come next, each a pass over the corpus rather than a place work is done. A
folder is walked to see what arrived, and a duplicate is settled once. Scan and Settings are last,
the two that change how the tool behaves rather than what the corpus holds.

**How often a page is opened is deliberately not the order.** Scan is wanted for every new folder and
still sits at the end. An order tracking how often a page was reached for would move as a corpus
matured. A strip whose items change places has to be read rather than aimed at.

## The corpus is browsed by folder

**The Folders page lists the corpus as its folders, each with how many distinct songs sit in it and
beneath it.** A folder links to the songs list narrowed by `?folder=`, and a song page links each copy
to the songs beside it. A corpus arrives sorted into folders by whoever collected it, so a folder is
how somebody sees what a new batch of files brought. No title, artist or tag filter says that.

**A scan that writes something rebuilds the tree before it finishes.** The rebuild is a whole pass
over every file, measured at thirteen minutes over a whole corpus. It holds the writing connection,
so a star clicked meanwhile is refused. That is paid at the end of a scan, which is already a long
wait nobody sits through. The page then opens at once. **A stopped scan does not rebuild it**,
whether the stop came during the reading or during the rebuild itself.

Thirteen minutes after pressing Stop is not a stop. The pass is abandoned between rows and writes
nothing, so the last tree built stays. The page rebuilds an out-of-date tree itself when no scan is
running. While a scan runs, it shows the last tree built.

## Curation database

**Its own SQLite file, `km-package-builder.kmbuild`, in the folder being curated**, created only by
`--init` or the Open page's `Create here`. Not `library.sqlite`: song number keys that one, and it
holds what has been *installed*. Nothing here has a number yet, and most of it never will.
The analysis of a corpus stays beside the corpus. A second machine can be pointed at the same drive
and pick up where the first left off.

**The extension is `.kmbuild` because the database is the document you double-click** — see
`A corpus is a document`. The file is ordinary SQLite; only the name is special.

Opening looks for whatever single `.kmbuild` a folder holds rather than for a fixed name. A corpus
may be renamed `Brasil.kmbuild` and shown by name in a file manager. Two of them in one folder is an
error naming both, because no arbitrary choice is safe.

## A page is drawn through a connection that cannot write

**The tool stays answerable for as long as reading a corpus takes.** A scan is minutes at best and
hours over a whole corpus. It is the ordinary way the tool is used, not an exceptional state to be
locked out of. So browsing, searching and looking at a song answer throughout.

**So an open folder holds two connections on its database: one that writes, and one that only reads.**
Every page is drawn through the reading one. Write-ahead logging is what makes that safe. A database
that did not take it has one connection: the modes it falls back to are ones where a writer excludes
readers outright. There, a second connection answers *database is locked*, which is worse than
answering late.

**The status bar and a folder count are readings taken while the corpus is being read**, and may lag
a batch. They say how much of a corpus there is, not what is on the page. So a number a moment old is
the right trade against a whole-corpus count on every page of the tool.

A folder tree is not rebuilt at all while a scan is running. The marker it is checked against moves
on every batch. So every visit would pay for a pass over every file to produce counts the next batch
makes stale. The scan rebuilds it when it finishes.

**A curation action taken while a corpus is being read goes through, because the scan gives way to
it.** Nothing may write inside a batch, so a star clicked during the reading waits for the end of
one. The scan then stands aside instead of starting the next. Otherwise it would keep the database to
itself for the whole run, and every write would be refused. The cost falls on the scan, which is the
right side for it to fall on. A person is waiting on the star, and nobody is waiting on the batch.

**The exception is the end of a scan that changed something, and it is not a short one.** Rebuilding
the folder tree is one pass over every file, and measuring the corpus for the query planner is one
statement. Neither can give way part-way through, so on a whole corpus the two together run for the
best part of half an hour. A write arriving then is refused rather than delayed. Reads are
unaffected, and the page a person is looking at goes on working. Stop ends the rebuild between rows,
and the measuring does not start after a stop.

**A write that still cannot have the database says so rather than waiting.** The refusal travels as
a code with the page writing the sentence, as
`A refusal travels as a code, and whoever shows it writes the sentence` in `api-and-network.md` has
it. **A request that never answers is not an option**: a browser allows six to one host. Once they
are spent holding requests open, nothing else on the page loads either. That reads as the whole tool
having died, rather than as one action being late.

## A refused page keeps the navigation

**A refusal that replaces the whole window is drawn as a page, inside the ordinary layout.** It
carries the header, every tab, a control that asks again, and a link to the songs. A refusal that
arrives at a page still on the screen stays a status and a sentence. The browser puts it in the
corner.

**The two halves are one rule seen from either side: what is left to press.** A curation action
leaves the page it was taken from intact. The sentence has somewhere to land, and the tabs are
already there. A navigation throws that page away before the answer arrives. Answered the same way,
the window holds one line of unstyled text, with no header and no link. This tool's own window is a
webview with no address bar, no Back and no reload, so closing it is the only way out.

**The status says what happened, whatever shape the body takes.** A corpus that refuses a read
answers 503 for both. A 200 would tell a reload, a proxy and every later caller that the read
succeeded.

**A busy corpus asks again by itself, and nothing else does.** It is the one refusal that means *ask
again*, so that page carries a refresh. It comes back to what was asked for once the corpus is free.
A song that is not there would reload for ever and find the same nothing.

**Where a scan is running, the page draws its progress.** Those numbers are read from memory, so they
answer while the corpus does not. The scan is both the reason for the refusal and the thing somebody
turned away from it wants to watch.

**The header leaves out what it cannot read.** A page drawn because a read refused cannot ask for the
corpus counts or for the chosen machine. It shows neither, rather than four zeroes and a *no machine
set* that are not true.

## A count is answered from an index that holds what it counts

**`songs_countable`, an index on `merged_into` and `duplicate_of`, serves the browse page's total.**
Every filter carries those two terms, because a count has to narrow with the page it counts. Two
terms on two separately indexed columns defeat both of those indexes, so the count read every row of
the widest table in the schema. Measured on a whole corpus with the cache cold, that was thirteen
seconds for a number that labels a page whose rows took one millisecond.

**The index holds the columns rather than being partial on the identifier.** That is the opposite of
the three partial indexes beside it, and the difference decides whether it works at all. A partial
index still has the generated code read the terms off the row. So the planner correctly refuses it as
an index scan plus a lookup per row. Holding the two columns makes the count answerable without
touching a row.

**Selectivity is not what makes it worth having, and would condemn it anywhere else.** Both columns
are NULL on nearly every song, so the seek returns nearly everything. What makes it cheap is that two
narrow columns are a fraction of the width of the table. A corpus pays one pass and a fresh
`ANALYZE` for it on the first open. An index with no statistics is one the planner will not choose.

## Where the tool's own output goes

**A `_kmbuild-data` folder inside the corpus, holding packages, descriptions and backups — and not
the database.** Loose in the corpus root is tidy on a folder of forty songs and unusable on a large
one. A package built last week is three files somewhere in the least navigable directory
on the drive. The leading underscore sorts it away from the songs, because a corpus is somebody else's
folder that this tool is a guest in.

**The database stays in the root, and that exception is the whole of the design.** A `.kmbuild` is
the document you double-click. So moving it one level down would make a corpus a folder that cannot
be opened by opening anything. The distinction is not arbitrary: the database *is* the corpus, and
the other three are things made *from* it.

**Nothing has to skip the data folder.** The scan collects by extension, and `.kmpkg`,
`.kmspec.yaml` and `.kmbackup.json` are not among the extensions it collects. So a data folder inside
the tree being scanned cannot enter the catalog. An exclusion list that has to be kept in step with
the writers was never needed.

**A relative path typed into the import or restore box resolves there.** The three defaults write
into the data folder unconditionally, so that is the one place a bare name can mean. A name matching
nothing reports the data folder's path, because that is where to go looking. An absolute path is the
person's own and is used as given.

**The backup is something to copy off the drive**, which the settings page says in as many words. A
default has to be *somewhere*, and beside the thing it describes is the only place that needs no
explaining.

## How a corpus is opened

**From a page inside the tool: recent folders, a folder browser behind a button, and a path box —
never a native dialog.** A browser cannot hand a page a folder — a file input gives contents, not
locations — so the choosing is done server-side. That costs nothing: the tool is on loopback and
already opens files and writes packages as whoever ran it.

A native dialog was rejected on two grounds. It needs a GUI toolkit on every platform for one
interaction, and it cannot show the two things that actually save time. Those are the folders you
opened last week and a badge saying which ones are already indexed.

**The folder is swappable while the tool runs**, so moving between albums is a click. On macOS that
is a requirement, not a convenience. A second double-click is delivered to the process that is
already running, rather than starting another.

**Opening is a polled job, not a request.** A large corpus runs migrations, builds indexes and
finishes with a full `ANALYZE`, which is minutes on a corpus-sized database. The rule that none of
that may happen before the socket is listening would otherwise be reproduced one level up, as a
request that hangs.

**A polled panel owes a changing sentence and a climbing count, and a job that is silent is a fault
rather than a quiet success.** A poll every half second is worth nothing if what it puts on the screen
is identical each time. The window it has to fill is minutes: to somebody watching, a fixed sentence
and a hang are the same picture. So every stretch of an open says which one it is. That includes the
version ladder and each repair after it, not only the two that were easy to count. Beside the
sentence runs a count of seconds, which goes on moving through the one step that cannot say anything
more specific than its name.

**An open lists all eleven of its rungs, and a rung it did not need is one it climbed past.** The
reason is the one
[`A scan shows its steps`](#a-scan-shows-its-steps-what-is-left-and-can-be-stopped-from-the-page)
gives. A name for the running step says neither what is to come, nor whether a step showing
the same words for five minutes is working. Where the two jobs differ is how a step that does not
run is marked.

A scan knows its skips at the gate that decides them. An open's gates are conditions inside the open
itself, with no page in reach. So reaching a later rung is the only thing that says an earlier one
did not run. **That makes the dash mean two things, told apart by where it sits.** Above the rung a
failure stopped at, a rung the open passed and did not need. Below it, a rung it was never going to
reach.

**An open that fails keeps its list, because a reason alone does not say which rung broke.** That is
the difference between a database that would not open and one whose index has been rewritten
halfway. The second is the state the folder is left in. **A refusal before any rung was climbed
shows no list at all** — the folder is not there, it holds no database, it holds two. Eleven rungs
marked *not needed* would say the open had considered each one and declined it.

**The bar is a proportion where the rung counts what it does, and says only *working* where it does
not.** Nine rungs are one statement each — an `ALTER TABLE`, a `CREATE INDEX`, an `ANALYZE` — with
nothing inside them to count. A partial bar beside one of those is a proportion nobody stated. It is
drawn full and resting rather than animated from nothing, since a browser may throttle, disable or
never paint it. The two that fold titles and read words go in chunks and know how far through they
are. Those are the two that take the minutes.

**What none of it may do is name work it is not doing.** A folder that opens in a moment passes
through the same rungs too quickly to read, and that is fine. Promising it several minutes of index
building is not. The console keeps a size threshold the page does not, because a block of three lines
about spinning disks is noise in a terminal. The page has no quieter thing to show instead.

**The folder being left is closed before the next one is opened, and one process opens a file once.**
The connections an open folder holds are the two
[`A page is drawn through a connection that cannot write`](#a-page-is-drawn-through-a-connection-that-cannot-write)
describes, and opening the *same folder again* is what this forbids. It is an ordinary act — the
Recent list offers it, and a second double-click sends it. Doing it the other way round runs the
migration on a second set of connections while the first is still live. It then closes the first with
a `wal_checkpoint(TRUNCATE)` at the moment the list is first asked for. A read meets a writer and the
answer is *database is locked*, which pressing the folder again cures, because by then the checkpoint
has finished. **A symptom that one retry cures is the shape a fault hides in.**

**An open that fails partway therefore leaves nothing open.** Everything answerable without touching
a database is answered before anything closes — the folder is not there, it holds no database, it
holds two. So the folder somebody is looking at survives a mistyped path. What it does not survive is
a database that turns out to be unreadable. A folder whose index has just been half-rewritten is not
one to go on curating.

**A migration folds its journal back before it says it has finished.** The write-ahead log a large
rewrite leaves is about the size of what it rewrote, and something pays to fold it back whatever
happens. The decision is only *when*. Inside the window the Open page is already reporting, the
connection that wrote it does the fold with nobody reading. That is the moment it costs nothing to
anybody.

**A connection that is not in WAL says so in the log.** The switch needs an exclusive lock and is
tolerated rather than required. An in-memory database has no journal, and a network share refuses
one. But the mode it falls back to is the one where a writer excludes readers outright. Two
acceptable reasons and one fault produced the same silence.

## The folder browser is offered, not drawn

**`Browse this computer…` is a button, and no directory is read until it is pressed.**

**Drawing it open reads a directory on every arrival.** The directory it reads is the one the
picker starts in: the corpus's own folder, so "switch to the album next door" is one click. On a
working machine that folder holds hundreds of thousands of files. The picker is also the page a
*failed* open comes back to, so the read happens again on the arrival somebody is least
patient about.

**It also puts a screenful of the wrong thing between the two lists people use.** Recent folders is
how a corpus is reopened, and the path box is how a new one is named. Walking the disk is the third
way, and the one that wants a deliberate press.

**`GET /open/list` shares the page's own default.** With no `at` the page falls back to home, and
that route falls back to the drives. Nothing notices, because the listing is always rendered with the
page's answer already in it. The button sends no `at`, so the two have to agree. `?drives=1` is the
⏶ crumb, and stays the explicit way to ask for the drives.

## A folder of thousands is a page and a box

**The browser draws a page of folders and offers somewhere to type, rather than every folder there
is.** A machine has folders with thousands of subdirectories in them. The system temp folder is one,
on the way to nowhere anybody wants. A walker that draws all of them is a list nobody can reach the
bottom of.

**The reason is the disk rather than the markup.** Each row carries a badge saying whether that
folder has been curated, and answering that means opening the folder and reading it. So a listing
costs one directory enumeration per subdirectory, drawn or not. The folder above is the one somebody
is passing through, rather than looking at. The page is what makes the question get asked a hundred
times instead of thousands. The names are gathered, narrowed, sorted and cut to a page, and only what
survives is asked whether it is indexed.

**The count beside the pager is exact, so it is shown.** The whole directory has been read to produce
the page, which makes the total a fact rather than a reading. That is the condition
[`How many rows a browse page holds`](#how-many-rows-a-browse-page-holds) already sets for saying a
number out loud. What is not offered here is numbered pages. Folders are arrived at by name, so the
tenth page of one is not a place anybody means to be.

**The box narrows; it does not search.** It matches part of a name in the folder being stood in,
ignoring case, and walking into a folder empties it. A name typed here is about the folder it was
typed in. It sits outside the part that its own result replaces. A box replaced a moment after a
keystroke is a box that cannot be typed into.

**Hidden and system folders are not walked to.** `AppData` and its kin are most of what stands
between somebody and the folder they are heading for, and nobody keeps karaoke files in one. A
dot-prefixed name is the portable half of the test, and Windows keeps the rest in an attribute. That
is the only place `AppData` says what it is. This hides them from the *walk* and from nothing else.
The path box opens a hidden folder by name, and a corpus already inside one still reopens from the
recent list.

## Song identity in curation

**The content hash of the file's bytes is the song's id.** One choice satisfies both requirements at
once: the same file always yields the same id. Byte-identical copies land on one song row, with no
grouping pass to run or get wrong. Near-identical files are a separate question, and the answer is
[`Duplicate aggregation`](#duplicate-aggregation): they are grouped, and the group is never a queue.

## Duplicate aggregation

**Identical files group by construction, the tool groups near-identical ones, and neither is
work anybody is asked to do.**

**The first half needs no feature.** A song's id *is* the content hash of its bytes — see
[`Song identity in curation`](#song-identity-in-curation). Byte-identical copies land on one row, and
how many there are is a column on it. `Copies, in the song row` is where a curator reads it.

**The second half is a group and never a pair.** Pairs are proposed, then joined into groups of
files that all look like one recording, and each group keeps one. One song paired with five others is
five rows saying one thing: on a large corpus, 36,462 pairs are 14,938 groups.

**Two signals propose a pair, and the second exists because of what the first cannot see.**

- **A structural fingerprint, confirmed by a matching name.** The confirmation is not optional: a
  corpus built from one studio's template would otherwise pair with itself. It is also the limit — a
  great many files are called `EARTHW~2` or `TRACK01`, and no name match can ever join them.
- **The song's words, needing no name at all.** Two files that sing the same words are the same song
  whatever they are called. That reaches the files the first signal is blind to. It found 4,219 more
  songs to set aside on the real corpus. It joined sets like *Down At The Twist & Shout* to a file
  called *Howdy You'all*.

**A lyric is keyed only after its credit lines are dropped, and only if 25 words remain.** A great
many files carry the sequencer's business card in the lyric track and nothing else. Keyed as sung
words, one such card made a single group of 126 unrelated songs. `km_suitability::is_credit_line` is
the rule that already decides this for the suitability score, borrowed rather than restated. The word
floor is measured: with the credits gone, the false groups left were legal boilerplate of 17 and 20
words. Every true group had 90 or more.

**Words reach only the files that have any** — 17% of MIDI songs. So the two signals
complement each other, and neither replaces the other.

**A group is not reviewed, and a review queue must not be added.** Three reasons, in ascending order
of force:

- **A count that cannot reach zero is not a queue of work**, it is a permanent number in the
  navigation. Thousands of groups is thousands whichever way they are listed, so nothing lists them
  and nothing counts them on screen.
- **A whole-corpus pass belongs to somebody who asked for it**, not to every scan that found a
  changed file. Two things ask: the button on the Duplicates page, and **Re-analyze everything**.
  That ends with the pass, because it has just rewritten every fingerprint the pass compares. So the
  grouping it would otherwise leave standing is grouping by shapes that are no longer there. Reading
  the corpus takes hours and the pass takes seconds.
- **The judgment is one the tool cannot help with, and mostly one nobody needs.** 89% of groups have
  their top suitability tied — the files are equally good and any will do. Where they are not tied,
  the tool already knows which is better. Listening answers *Is this the same recording?*, so it is
  asked where both files can be played: on a song's own page, beside each version.

**Which file of a group is shown is decided, not asked.** Best suitability, then most copies on disk,
then the id — the last not a preference but a promise. A representative that moved between passes
would reshuffle the song list for a reason nobody could see.

**Duplicates cost nothing in a corpus and cost work in a list somebody built**, which is where this
acts. A favorites list says how many of its entries are a second file of a song already in it. It
offers to drop them, keeping the best copy *that list holds*. A package says so as songs are added.
Before this, 30.6% of one real favorites collection — 2,171 memberships of 7,098 — was a song filed
twice.

**A pass replaces the proposals and never the verdicts.** The suggestion list is the tool's own
reading of the corpus. So what a pass is handed is the whole of it, and a pair it no longer proposes
is taken out. Leaving one in would let a group rest on a resemblance nothing still finds, because
grouping reads the table rather than the pass. A verdict is a person's word on a pair and outlives
every pass, which is what makes a dismissal hold.

**`duplicate_of` is not `merged_into`, and the two must never be conflated.** Both hide a song from
browsing, and only one of them is somebody's word. `merged_into` is a person saying two files are one
recording; `duplicate_of` is the tool's guess. A guess filed under the other would take a song away
with nothing to say who did it. So a backup carries the merge and never the guess. What a person
typed is the *dismissal* of a pair, and that travels with the verdict.

**Every pass rewrites the groups from nothing.** A song whose pair was dismissed, or whose fingerprint
no longer matches, comes back rather than staying hidden by a grouping nobody can find.

## The package builder's window

**A webview window on Windows and macOS; the default browser on Linux.** A tool that opens a browser
tab and holds a terminal open is not a desktop application. So `wry` and `tao` put the same page in
a window with the product's icon, and that window quits when it is closed.

**Linux is deliberately excluded**: `wry` links libwebkit2gtk at load time. A build carrying it does
not *start* on a machine without that library, and `--browser` cannot rescue it. The failure is in
the dynamic loader before `main`. That would trade this tool's best property, one executable copied
anywhere, for a window on the one platform where launching a browser is universal. The cargo
feature is off by default, staging turns it on for two platforms, and `--browser` declines it
anywhere.

**`tao` rather than a plain window library.** On macOS a double-clicked document is delivered as an
Apple Event to an application delegate, not in `argv`. So on that platform the event loop is the
mechanism, not the decoration. It is also why a second double-click there reaches the running
process.

**A window satisfies `--open` rather than competing with it**: the flag asks to be shown the page, and
the window is the page being shown. So a build that has one never also opens a browser tab.
`--browser` is how you ask for a tab *instead of* a window.

**The header draws exactly one of *Quit* and *Open in browser*.** Closing the window already asks for
the identical shutdown Ctrl-C and `POST /quit` ask for. So a Quit button is a second X beside the X.
A page offering two ways to do one thing invites the reading that they differ.

What a window genuinely lacks is the other direction: a webview has no address bar, no bookmarks and
no second window. So somebody who wants the tool in a real browser has no way to find the address.
`/quit` stays a route regardless — every browser build and every Linux build uses it.

**The window decides which one is drawn, not the flags that asked for one.** A `desktop` build whose
webview cannot be created falls back to a browser (Windows Server, an LTSC build with no WebView2).
That run has no window to close and possibly no console either, so it must keep its Quit. The flag is
set inside `desktop::run` after the webview is built.

## Reopening the last folder

**With no folder named, the tool reopens the one it had last; `--pick` asks for the list.** Most
people curate one corpus, so a picker on every start would relocate the friction rather than remove
it.

Only the most recent entry qualifies, and only when it is still a folder that still holds a database
this build can open. An unplugged drive and a folder whose database has been deleted both fall
through to the picker, which is where each is explained.

**The row and the reopen read one enum**, which is
[`One spelling per concept, across every surface`](foundations.md#one-spelling-per-concept-across-every-surface)
applied to a predicate rather than a name. A row asking only *is the folder there* would draw a
folder still on disk whose `.kmbuild` has been deleted. It would show that folder as an ordinary
openable row, counts and all. The reopen, which tests for a database this build can open, silently
declined the folder the page was offering as the obvious thing to press. Pressing it would produce a
refusal, and afterwards the row would look exactly the same.

**Three states, and only one of them is a button.** An action whose one outcome is a refusal is worse
than no action. The refusal arrives after the press, and nothing about the row has changed. So the
other two are text, and each says which it is — *not found*, *its curation database is gone*.

**The last of those is deliberately not *not found*.** The folder and its songs are still on the disk;
what went was the curation — the tags, the packages, the favorites. Somebody who reads *not found*
goes looking for a missing drive. And it gets no button beside it, unlike Adopt. Making a database
again means indexing the corpus from nothing, which is what `Create here` in the box below already
does. That is not a thing to put one press away.

**Adopting is deliberately never automatic**: it renames somebody's whole index, which is not a thing
to do because they double-clicked an icon.

Opening by name records the folder too, not just choosing it on the page. Without that, the list
would only ever remember one of the two ways in. Anyone whose habit is the command line would be
offered an empty one.

## Whose recent list a run writes

**`KM_PACKAGE_BUILDER_RECENT` names the file this run's recent list lives in, and set-but-empty means
it keeps none.** That means no remembering, and no startup reopen either, since both read the same
list. Unset is the per-user config file, so nothing about an ordinary run changes.

**Why it is needed.** The list is deliberately never pruned of folders that have gone missing: an
unplugged external drive must not lose a year of curation. So anything that gets into it stays
until somebody clicks `×`. At twelve entries an unwanted row is an eviction rather than clutter.

Dead rows accumulate, and **not from `cargo test`**: the `cfg(test)` guard works, and the population
it guards is drawn too small. They come from screenshot runs, from worktree verification corpora and
from agent scratch folders, all opened through the production binary. There, a `cfg` says nothing at
all. The write is unconditional and happens before any scan, which is why every junk row reads
`0 songs · 0 files`.

**An environment variable rather than a flag, and the file rather than the directory.** The runs that
must not write here are the ones a script starts. A script always has an environment where it may
have nowhere convenient to put one more argument. `--pick` is not the answer either, because it
declines the reopen and not the recording. Naming the file keeps the meaning exact: the log files and
the webview profile are in the *data* directory and do not move. It gives "nowhere" a state the type
already had.

**The `cfg(test)` guard stays and is checked first**: a variable is a thing somebody has
to remember, and a `cfg` is not.

**`KM_PACKAGE_BUILDER_PASSWORDS` is the same variable for the remembered machine passwords**, with
the same three states and the same `cfg(test)` guard ahead of it. The stake is higher rather
than merely equal. A screenshot run or a smoke test that wrote into the recent list cost somebody a
row. One that wrote into the password file would be putting a credential into a store the owner
believes they control.

Two variables rather than one, because they move two files that hold two
different kinds of thing. `Where a key somebody typed into a page lives` in
[`repository.md`](repository.md) is the rule that keeps credentials in a file of their own. One
variable covering both would undo it from the other end.

## A corpus is a document

**The curation database carries a registered extension, `.kmbuild`, and double-clicking it opens the
corpus.** A folder of karaoke files is the unit of work here, and the alternative is typing its path.

Associating `.sqlite` was never an option: it would claim every SQLite file on the machine, which
belongs to nobody. So the database takes an extension of its own.

**The database itself, rather than a sidecar pointing at it.** A marker file is a second thing
to keep in step and a second thing to lose. The path arrives differently on each platform, and the
difference is invisible to everything past `main`. Windows and Linux pass the file in `argv`; macOS
delivers it as an Apple Event after the event loop starts.

## A path in a `.kmbuild` is a claim, and it stays under the corpus folder

**Being a document is what makes this a rule rather than a nicety.** A `.kmbuild` opens on a
double-click, so it is a file somebody may have been *sent*. `files.path` is then the sender's
word about what to open, not the tool's own writing. The schema asks for a path relative to the
corpus root and asking is all it can do.

**What the routes do with the answer is the reason.** A song's file is read, downloaded to the
browser, and handed to the system opener. When the machine it is pointed at is not on this box, it is
**uploaded to that machine**. The address of that machine is a setting in the same database. So one
document names both a file and where to send it. That document reads and posts anything the person
who opened it can read.

**`Db::best_file` is where it is caught**, because every one of those routes reaches a file through
it. An absolute path replaces the root when joined, and a `..` component walks out of it. So both are
refused rather than repaired: a database that says one of these is a database saying something untrue
about itself. Opening the *nearest legal file instead* would be a worse answer than opening
nothing.

**A symlink below the root is still followed, deliberately.** Refusing one would mean canonicalizing
every lookup, and a corpus kept across two drives through a linked folder is an ordinary way to keep
one. The rule is about what the *document* may say; a link is something the corpus's owner made.

**A package member's path is dropped rather than refused.** It is the one reader where that is
right. A member with no usable path is already the *its source file is gone* case a build reports per
song. So an unfollowable path lands in a report somebody reads instead of failing the build.

## A song with no title

**Shown under its own file name, without the extension, and marked as such.** Most of a real corpus
carries no title meta event at all, so the honest rendering is an empty cell. That turns the first
pages of a full-corpus scan into page after page of blank, unclickable rows. All are sorted to the
front because the empty string sorts first.

The file name is the only name those songs have. It is what a curator is shown, what sorting orders
by, and what the search box matches. It carries a `file name` tag saying where it came from.
Telling a song somebody has checked from one nobody has is most of what curating is.

**A song whose file named nothing but marks arrives here too**, by
[`A name made of marks is not a name`](songs.md#a-name-made-of-marks-is-not-a-name). A separator row
in a title meta event is refused rather than kept, so the row falls through to this one.

**The tag is the whole signal: the title is not dimmed.** In a list of blue links a gray one reads as
*visited*. That is a stronger and quite different claim than *this song has no title of its own*.

It is stored in its own column and never in `det_title`: nothing inside the file said it, and "what
did the file say?" has to stay answerable. **No artist is ever invented** — a missing artist stays
missing.

**And it is what the edit box opens with, not only what the list shows.** A field holding a value
nobody typed invites somebody to accept it, which is why `det_title` is a *placeholder* on the song
page. But the asymmetry is the point: that box sits beside a row already stating the detected title.
In the list, the name is the one thing on screen. So clearing it deletes the only name the song has
at the moment somebody is trying to correct it. The commonest correction on this corpus is turning
`CORCOVAD` into `Corcovado`, which otherwise begins by retyping `CORCOVAD` from the link just clicked.

**The artist box has nothing to prefill it from**, so it stays empty. Saving files the name under
`title`, exactly as the bulk *Title from file name* action does. So the row gains its `ed` tag, and a
rescan will not take it back.

**The bulk action takes the artist with the title, and the song page does not.** One pass of one
sequencer wrote both fields. So a file whose title says `UNTITLED` names a tool, a studio or whoever
typed it in the artist beside it. A curator who has just replaced fifty titles would otherwise
walk the same fifty rows again. The song page is the other scope by the argument
[`Acting on a whole filter`](#acting-on-a-whole-filter) already makes. Somebody there is looking at
one song and can see whether its artist is worth keeping.

**It is written as the empty string and not as NULL**, which is the same distinction one column over
and settles the opposite way. `eff_title` folds the two together with `nullif`. `eff_artist`
deliberately does not, so NULL falls back to what the file declared, and only `''` stands in front of
it. This does not invent an artist — it records that nobody has named one, which is what the browse
list's dash has always said. The list sorts blank before absent, so a batch done this way arrives
together where somebody can look at it.

## File names in the browse list

**A box in the Songs filter bar shows every row's file name beside its title, off by default.**

A title that *was* detected can still say less than the name somebody typed on disk. That name is
otherwise only reachable by hovering each row one at a time.

It shows the **base name with its extension** (`Corcovado - Tom Jobim.kar`), not the path. The folder
is the Folders page's question, and a column of paths would be unreadable at this width. Where a song
has several byte-identical copies, it names **the same one the hover names** — the prettiest, by
`nicest_path`. So the two cannot disagree about which file is being talked about. It is **suppressed
for songs whose title already is that name**: repeating the whole title to add an extension is not
information.

The box sits with the filters, although it narrows nothing: it is turned on and off while
browsing and has to survive a page turn. It is **not** persisted, though: it belongs to the browsing
somebody is doing now, not to the folder.

## What the analysis found wrong, in the browse list

**A second box beside the file-name one shows every row's warnings, off by default.** They are what
the analysis had to say against a song — `no_lyrics`, `poor_lyric_sync`, `line_level_lyrics`.
Otherwise they are on the song's own page alone, so finding the defective files in a folder means
opening songs one at a time.

**The code is the chip and the message is its tooltip**, which is how the song's own page draws the
same two fields. A code is short enough to scan down a page of fifty and specific enough to be
looked up. The sentence is what somebody reads once they have found the row.

**Last in the title cell, after every other chip.** It is the one mark that can come several at a
time. It is also the only one that is a judgment about the file rather than a name for it. So a row
with three warnings pushes nothing a reader was using off to the right of them.

**Off by default, for the file-name box's reason with a column that is empty on most of a corpus.**
The width comes out of Title and Artist until the table scrolls sideways, and the pass this is for
is hunting defects rather than browsing. Like that box, it narrows nothing, so it keeps the page and
survives *clear all*, and is not persisted. It belongs to the pass somebody is making now.

**Nothing filters or sorts by them, and that is deliberate.** Suitability is the number that already
orders a corpus by how much is wrong with each file, and it is built from these same findings. A
second control over the parts would offer a curator two answers to one question. This shows what the
number is made of on the rows the number already ranked.

## Copies, in the song row

**Hovering a title lists every folder the song sits in, headed by how many there are.** Showing only
the one copy whose name looks most like a title hides that the others exist, and where they are.

The count goes in that tooltip rather than beside the title: the Copies column already carries the
number. So a badge next to the name is a second marker for something already on screen. The hover is
the only place that can answer *which folders?*.

## When a song was last edited

**Every song carries the time somebody last changed it, and a song nobody has changed carries
nothing.** Unset is NULL and not a date, the rule a rating follows. A song nobody has edited and a
song edited at the epoch are different facts. So the sort keeps them apart by putting the unset
ones last.

**An edit is a decision somebody made about the song's own record, and a scan is not one.** The
names, the language, the pinned encoding, the transpose, the notes, the rating, and a merge. What
a rescan rewrites is what each *file* says about itself, which answers nothing about when anybody
last curated it. So a corpus re-read from disk reads here exactly as it did before.

**A tag and a favorite are filed in tables of their own and leave the stamp alone.** That is the
one place this is narrower than what a backup carries. Filing a song says where it belongs. The
stamp answers *what was I working on*. A tagging run over a whole filter would otherwise bury
the twenty songs somebody spent the evening naming.

**The browse bar sorts by it, most recent first, and the browse list has no column for it.** The
question it answers is an order rather than a field. A row already carries as much as can be
read at a glance. One more date on it would cost the reading of everything beside it.

## When a song was added

**A song's added date is the time a scan first found it, and no later scan moves it.** It is
`songs.first_seen`, so every song has one. A song whose last file leaves the corpus is forgotten, and
if the file comes back the song is added again with a new date.

**The browse bar filters by it in four bands counted back from now:**

- the last day
- the last 7 days
- the last 30 days
- more than 30 days ago

The first three are nested, because *what arrived this week* is a question that includes today. The
bands are relative, so a saved link keeps asking the same question tomorrow.

**The bar also sorts by it, newest first, and the song page shows the day.** The list has no column
for it, for the reason given for the last-edited date.

## Searching lyrics

**The curation tool stores every song's words and searches them on a page of their own.** On a corpus
where most files carry no title meta event, the words are often the only thing about a song that is
known. *Which song goes like this?* is a question the title box cannot answer.

It is a **separate page, not another filter on the browse list**. An FTS5 `MATCH` searches every
column of its index. Folding lyrics in beside title and artist would turn the "title or artist" box
into a lyric search. Type `love` and half the corpus comes back, because half the corpus sings the
word. Two questions, two indexes.

A hit is drawn as an ordinary browse row with the matching passage beneath it. A song found by its
words can be scored, starred, played or filed where it was found. The scan writes the words, so a
corpus indexed without them has none until it is re-read with `--force`. The page says so rather than
reporting no match.

**The hits page pages the way the browse list does**, numbered, with a window either side and a jump
to each end. The two lists are the same act at two indexes. Placing yourself in a list too long
to hold in mind is the same problem in both. It matters more here, if anything: the order is how well
the words matched. The page where a half-remembered line stops being the best answer and starts
being a coincidence is a place somebody goes back to. A strip of *next* buttons is as many
presses as there are pages to reach it.

A hits page holds 25 where a browse page holds 50, because a hit is two rows rather than one. A
screen of passages is read rather than scanned. Nothing else about the two pagers differs, and the
numbers behind both are worked out once. A lyric total is an exact count, so the *scanning* tag and
the clamped last page the browse list needs have nothing to say here.

## Quotation marks ask for the words in that order

**Two words typed in either search box match anything holding both. A pair of `"` narrows that to
the words in that order and next to each other.** Loose words are right for a half-remembered
fragment, which is what most searching here is. They are wrong for a line somebody can actually
quote. Over a corpus this size *quiet nights* is thousands of songs with the two words
nowhere near each other. Both readings are wanted and only one can be the default, so the other is a
mark somebody types.

**Both boxes, not just the lyric one.** The two indexes answer different questions, but *these words,
in this order* is the same request over either. A punctuation mark that worked on one page and
was silently ignored on the other would be worse than not offering it.

**An unclosed mark closes at the end of what was typed, and keeps the prefix match the last word
always has.** The box answers as somebody types, so a phrase is unclosed for as long as it takes to
type it. The alternative is a query meaning something else entirely until the second mark lands, with
the results jumping and then jumping back. A phrase that *is* closed takes no prefix match, because
closing the marks is how somebody says they meant exactly that.

**A `"` is never anything but the mark.** It opens or closes a phrase and never reaches the
expression, so the guarantee the quoting exists for is unchanged. `OR`, `*` and an apostrophe are
still matched literally, and no typed text can become syntax.

## Finding a song under another spelling of its name

**Every browse row and every song's own page carry a ≈ button that lists the songs with a similar
name, likeliest first.** A corpus files one song under many spellings:

- `Springsteen, Bruce`
- `Dancin' in the Dark`
- a publisher's `[SF Karaoke]` on the end
- the artist and title swapped
- both inside a file name

The title box asks for every word, so it reaches one spelling at a time. The
duplicate pass reaches only files whose structure or words match. Whether a better file of a song
exists is a question asked while looking at one. The button is on the row and the page for that reason.

**The search is loose on purpose, and it ranks rather than filters.** A false match costs a glance.
A missed match is the file somebody was looking for. The likeness is a column before Artist, so the
point where the list turns into other songs can be seen. It is green above 90%, which is where a
match is almost always the same song.

**What counts as a similar name:**

- Case, accents and punctuation are folded, as `km_song::text::fold` folds them everywhere.
- What brackets hold is left out, because it describes the file rather than the song.
- Articles, short joining words and words such as *karaoke* and *version* are left out. Two
  unrelated titles share them often enough to lift one toward the other.
- A word matches a word it begins, at four letters or more, and a word that shares most of its letter
  pairs. That reaches `Dancin` and `Springstein`.
- Where both names carry an artist, the title weighs three times the artist, because a cover by
  somebody else is still the song.
- An artist only one side names does not count against the match. A search with no artist is judged
  on the title, and a file with no artist is weighed like a cover. Otherwise a one-word title searched
  alone misses every file that names a two-word artist.
- Title and artist are also compared as one pool of words, which reaches swapped fields and file
  names. A pooled comparison counts only as much as it finds of the title. Otherwise every other song
  by the same artist ranks beside the one wanted.

**Candidates come from the title and artist index, and the score is worked out outside SQLite.**
`songs_fts` is asked for any word's first five letters and gives the 400 best by `bm25`. Those are
scored in Rust, and the matches are the ones at 40% or more. No tokenizer compares one word with
another, and scoring every song in the corpus reads the whole table. Over the real corpus a search
answers in about a tenth of a second, and in a few seconds on a cold disk.

**One page of at most a hundred, with no count and no pager.** Past a hundred, a list ordered by
likeness is coincidence, and counting the rest would mean scoring every candidate the index holds.

**The matches are browse rows**, for the reason lyric hits are: a better file is starred, filed or
played where it is found. The title and artist boxes stay editable, so somebody can loosen a name by
hand when it is garbled past matching. A file merged into another is left out.

**The ticked matches take five of the Songs page's actions:**

- **filing into a favorite**
- **the quality hint**
- **_Title from file name_**
- **_Fix the capitals_**
- **_Artist from the title_**

The list is the files of one
song side by side. That is where setting the better file aside, choosing the one to play first and
correcting a garbled name are done. The three name actions redraw the matches for the search in the
boxes, so a renamed file shows its new name where it was ticked. The Songs page's remembered
filter is left alone.

**They are three tabs in the Songs page's strip: Quality, Favorites and Titles**, for the reasons
[`The curation actions are tabs`](#the-curation-actions-are-tabs) gives. The three name actions share
the Titles tab, as they do there. Quality comes first and opens first, because choosing which file of
a song to play first is what a match is most often found for. The head of the list carries the Songs table's box, which ticks
every match.

**The page narrows by suitability, media type, lyrics, copies and versions**, with the Songs page's
controls, bands and field names. These are what separate one file of a song from another. The filters
narrow the candidates before any is scored. A match the filters keep is still reached when unfiltered
files would crowd it out of the 400. The song the search started from heads the list whatever the
filters say, because every other row is read against it.

**A recording shows one row unless *every version* is ticked**, as on the Songs page. A file the
duplicate pass hid as a version sits beside its primary under nearly the same name. The primary
already stands for it: its versions count leads to the rest. Ticking *every version* is for choosing
between them by ear.

**The five filters open as they were last set, for as long as the tool runs.** Curating is a run
through many songs looking for the same kind of file, and a ≈ link carries only the name. An address
naming any of the five is used as sent, so a filter set back to *any* stays there. They are kept in
memory and survive a change of folder, because a band or a media type names nothing out of a corpus.

**Suitability opens at 8–10 until somebody sets it, and the other four open at *any*.** A match is
looked for to find a better file of a song, and a file under 8 is seldom that one. Setting it to *any*
holds for the run like any other filter.

**The song the search started from comes first, whatever its likeness, and its likeness cell is
marked.** Every other row is read against it: which copy is longer, better, or already filed. The page
fetches it by id rather than finding it through the index. A name edited in the boxes can push it out
of the candidates or below the threshold.

**It keeps that row when no name is like it, under the sentence saying so.** The page answers about
one song, and a sentence with nothing under it reads as a page that failed to load.

**It writes nothing, and it is not the duplicate pass.** Two files with one name can be two
recordings, so listening answers whether they are the same song. A similar name is a reason to
listen. It is never a reason to group the files.

**A file whose only name is a short DOS name, such as `DANCIN~1`, is out of reach**, as it is for the
duplicate pass. No spelling of a title can be recognised in it.

## Finding a song by the words it sings

**Every browse row and every song's own page with words carries a ≋ button that lists the songs
singing the same ones, likeliest first.** It is the question the ≈ button asks, put to the whole
lyric instead of the name. It reaches the file that neither of the other two can: `DANCIN~1` has
no spelling to recognise. A copy with a verse missing keys differently from the duplicate pass's
exact lyric key. Between a name that says nothing and words that must match exactly sits a great many
of the corpus.

**Tight where the names page is loose, and that is the whole difference between them.** A similar
name is a reason to look at a file. The same words are close to a statement that two files are one
recording. The list is therefore a short one of near-certainties rather than a long ranked one. A false
match here would be read as fact.

**What counts as the same words:**

- The words are the sung ones: credits, addresses, the legal boilerplate and section labels are left
  out by `km_song::looks_like_a_banner`. That is the rule the duplicate pass already uses. A song
  with fewer than 25 words left says nothing and is not compared.
- Case, accents and punctuation are folded, as `km_song::text::fold` folds them everywhere.
- The match compares two songs by their runs of three words. One word is a bag that every song in the
  language fills, and a single stray syllable breaks a longer run.
- The score is the share of all the runs either song has that both of them have. A verse missing
  from one file therefore costs roughly what that verse is worth.
- **Not the share of the shorter song.** That reading makes a medley hold every song in it at 100%,
  and a file carrying one verse the whole of the song. Both answer *is this the same recording* with
  *yes* where the honest answer is *part of it is*.
- Where a file wraps its lines is not compared, because one sequencer wraps where the next does not.

**What it misses, on purpose**:

- a cover with reworked verses, which keeps its chorus and little else
- a medley
- a file whose lyric track holds nothing but the sequencer's business card

**Candidates come from the lyric index, and the score is worked out outside SQLite.** A dozen
three-word phrases, spread across the song so a file missing its opening still matches the rest, are
asked of `lyrics_fts`. The best 400 by `bm25` are scored in Rust. Asking for the words one at a
time instead would match most of the corpus, because most of it sings *love*.

**A phrase is chosen for how rare its rarest word is.** A phrase is only as selective as that word.
`ORDER BY bm25` ranks every row a query matches before any limit cuts one. A page asking for
whatever words a song happened to open with would have SQLite score a large part of the corpus. That
is what it would take to return a hundred rows. `lyrics_vocab` is what answers how rare a word is, and
costs no storage.

**A phrase never crosses a line the banner rule dropped.** The index holds the whole lyric, credits
included. The word before a dropped line and the word after it are not next to each other there. A
phrase built across that seam asks for something no file holds and quietly matches nothing.

**The address names the song and carries no text.** The similar-names page keeps its two boxes
editable, because somebody can loosen a name by hand when it is garbled past matching. A whole lyric
body is not something a box can hold or anybody would edit. So the song searched from heads the list, marked, and
every other row is read against it.

**It heads the list when nothing else sings its words as well.** The page answers about one song, and
a sentence with nothing under it reads as a page that failed to load.

**It narrows by the same five controls as the similar-names page, and remembers them separately.**
The controls mean the same things, so they are the same controls. What they open at differs.
A similar *name* is looked for among the files somebody might play, so that page opens at 8–10.
A file singing the same words under another name is most often the one nothing else could reach. It
is also rough enough to score under 8.

One record would let whichever page was opened first decide
what the other opened at. This one would say *no other song sings these words* with a match sitting
behind the band.

**The button is not drawn on a song with no words.** Most of a real corpus is instrumental, and a
button that can only lead to a page apologising is one on nearly every row.

**Three empty pages, because there are three different problems with three different fixes**:

- no scan has written any words yet, and the corpus is re-read
- this song has too few words to find another by
- nothing else sings them

The first two carry no row, because the song has nothing to be read against. The third carries the
song searched from.

**It writes nothing, and it is not the duplicate pass.** Two files singing one set of words can still
be two recordings. The same words are a reason to listen, and never a reason to group the files.

## Acting on a whole filter

**A curation action either takes the rows somebody ticked or everything the filter matches, and which
one is a statement about the judgment being made.**

**Ticked** is right when the judgment is about *that file*. Taking a song's title from its file name
is looking at `UNTITLED` and deciding the name on disk says more. Nobody can make that judgment a
filter at a time.

**Filter-wide** is right when the judgment is about a *set somebody has already
described*. Every song of an artist shares a language, and so does *these are the songs that go in
this package*. The size of a corpus is what makes the second kind worth building. The alternative
is a hundred pages of ticking to say one thing.

**The language set does both, and the ticked one is the default.** Filter-wide alone makes the small
version of the same judgment awkward: setting the language of three songs would mean describing them
in the bar first. *These three* is not a thing a bar of fourteen controls can usually say. The
two are one control with a scope select, because they are one judgment at two sizes. The
smaller is the safer default for a control that can otherwise write every row in the corpus.

**Recalculating the suitability does both, and is the one of these that reads rather than writes.**
A revision of the thresholds leaves every stored number describing the rubric that produced it.
Nothing in the database can recompute one: a suitability is derived from notes, channels and lyric
timings, and what is kept is the conclusion. So it re-reads the files, which makes it **a scan of a
named set**. It shares the same worker threads, the same progress bar and the same Stop button. The
button starts a job and says where to watch it rather than holding a request open for an hour.

**A scoped run concludes nothing about the corpus.** *Which files are gone* is what the walk found
subtracted from what the database holds. So a run handed four hundred paths of a whole corpus
would answer *everything else*. It does not ask, it does not group near-duplicates, and it does not
stamp the folder as scanned. Only a run that looked at all of it may say anything about all of it.

**What somebody typed is never at risk**, and that is inherited rather than promised. A scan writes
the `det_` columns and the analysis. The title, artist, language, rating and notes are the other
half of the split `schema.sql` draws.

**Filing into a favorite does both too, and names which way it goes.** A folder is as often the thing
somebody wants a favorite made of as it is the thing they want a language set on. The ticked-only
version meant a hundred pages of ticking to say one thing. The scope select is the language set's,
with the same ticked default.

**And it takes songs out as well as putting them in, because the direction is a word and not a
reading.** The objection this answers is real. One control that filed a song or unfiled it depending
on where it already was would be a single tick meaning two opposite things. An evening's filing
could go in a click nobody saw coming.

A named direction is not that. It is counted, it shows what it
would do to which favorite, and it is confirmed. At that point *take these out* is as ordinary as
putting them in. A list somebody over-filled is as much work to fix as one they under-filled.
`favorite_action` is the field, beside `tag_action` which settles the same question one tab over.

**The favorite is named in the confirmation**, because this is the last place to notice the wrong one
before a whole filter goes into it.

**Adding to a package that exists does both as well, and it is the one of these that does not always
ask.** A folder is as often the thing somebody wants put into a volume they are already building. It
is just as often the thing they want a language set on. The whole-filter half was otherwise reachable
only by *making* a package. So a second folder going into a volume meant a hundred pages of ticking or
a second package. The scope select is the language set's, with the same ticked default.

**Its ticked half is the one filter-wide control with no confirmation**, and what earns the exception
is that the set is already the answer. The rows are on the screen, ticked one at a time by the person
pressing the button. The route is also what a song's own page and the Lyrics page post to. Neither has
a filter to count or anywhere to put a question. The *matching* half holds all three rules
below, exactly as the four controls above it do.

**The confirmation names the room the package has left, which making one has no need of.** A package
made from a filter numbers from the start it was given. One that already holds four hundred songs
numbers behind them. So a filter of eleven thousand meeting a package with two hundred numbers free is
a fact. Somebody should read it before the write rather than in the sentence after it. What does not
fit is left out and counted apart, as it already is for a page of ticks.

**And it can be narrowed to the songs nothing has said a language for.** The obvious objection is
that the browse bar already has *language unset*. What the two controls do to the *list* answers
that. The bar narrows what is on screen, so the rows being looked at disappear. The ones meant
to be left alone are no longer in front of anybody.

This narrows the **write** — look at a folder,
classify what nothing has said, leave every judgment already made exactly where it is. *Unset* means
the same thing in both places by construction: the write asks `LanguageFilter::Unset` for its clause
rather than restating it.

**It is the last control in the row, and it says *only songs with no language yet*.** Between the
scope select and the *to* select, the labels of three controls run together left to right into
`only where nobody has said to Portuguese`. That is a sentence about something else entirely. Last, the
sentence completes before the box is reached and the box qualifies a statement already made.

*Nobody has said* is the house term for a NULL somebody could have filled in. That word choice is
exact in the schema, in `LanguageFilter::Unset` and in this file. On a control it asks a curator to know the
vocabulary before they can tick a box. The label says what the box does, in the confirmation chip's
own words: `no language yet`.

**The count is counted through the narrowing**, not taken from the number of ticks. With the box
ticked, most of a ticked page is usually classified already. A confirmation offering to set fifty
when it will set four is the kind of number that stops being read.

Three rules hold for every filter-wide action:

- **The filter travels in the query string for the confirmed write and in the body for the count.**
  Both halves have to be stated, because each of them read alone is a bug. A query string
  *rendered with the page* is wrong for the counting pass. The bar swaps `#rows` and never re-renders
  the page, so the moment somebody picks a filter that attribute describes a page that no longer
  exists.

  Narrow to seventeen thousand songs, press the button, and the server is asked about the
  whole corpus. So the count reads the bar live, out of the body, through `FilterQuery::from_body`.
  The *confirmed* write then uses the query string the confirmation itself wrote, which freezes the
  set that was counted. So it cannot drift between the number somebody read and the button they
  pressed.
- **The write reuses the browse list's own `WHERE`** verbatim, so what was listed and what was written
  cannot diverge.
- **It counts, shows the chips, and asks** before it does anything, because the filter that decides is
  fourteen controls further up the page.

Making a package **creates** rather than adds, so the name is turned into an id. The id is shown in
the confirmation, and a collision is refused in words. Adding names no id: the package was picked out
of a select, and what somebody saw there is its name. The ordering is load-bearing for both —
`add_to_package` numbers songs in the order it is handed them. So a filter sorted by title goes in
numbered by title, whether it is making the package or joining one.

## A stored analysis says which build decided it

**A curated corpus keeps conclusions and not the evidence they came from.** A build that decides
something new about a file cannot correct what is already stored. A suitability comes from notes,
channels and lyric timings; what is kept is the number. The only correction is to read the songs
again, and on the real corpus that is **four and a half hours**. That is measured at 20 songs a
second on a 7200rpm drive, with the process at a twelfth of one core. The whole of the cost is the
drive seeking between files that are nowhere near each other.

**So every song records the revision of the analysis that decided it.** A scan re-reads the ones
that disagree with the build doing the scanning. A file is skipped when its size and time match what
was already read, and the revision matches what this build would write for it. Two ways for a row to
be out of date, and only one of them is about the file.

**A file with no song records the revision too, on its own row.** A MIDI file that does not parse,
a readme `.txt` and the `.cdg` half of a pair have no song to ask. A large corpus holds a great
many of them. A skip test that asked only the song would read every one of them on every scan. That
is a drive seeking for as long as those files take to learn nothing. A limited reach never reaches such a file, because
a revision that could turn a failure into a song reaches everything.

**What this buys is that a changed heuristic reaches a corpus by itself.** *Scan changed files* is
the answer to "the rubric moved" as much as to "I added some songs". It costs what is actually
stale rather than a reading of everything. An interrupted run resumes, because each batch commits
the revision as it goes; a second run after a small change is nearly free.

**Somebody bumps `ANALYSIS_REVISION` by hand, and a test is what stops that being forgotten.**
Forgetting is otherwise silent: the corpus keeps reporting what an older build decided, and every
scan skips it because no file moved. Nothing anywhere says so.
`the_analysis_revision_covers_what_the_fixtures_say` hashes the tuning constants together with what
the analysis says about every fixture. So a threshold moved, a heuristic changed or a parser corrected
all fail it with the two lines to edit.

**It reaches as far as the fixtures do and no further**: a
behavior none of them exercises passes it. The answer to that is a fixture.

**A revision states which songs it can change, and a song outside that reach is not read again.**
Most heuristics touch one kind of file. A rule about chord names in the lyric track cannot change a
song with three lyric lines, or a video. `km_suitability::REVISIONS` gives each revision a reach, and
opening a folder raises every song outside it to the next revision without reading its file. A scan
then reads only what the change can reach.

**A reach names only a stored fact the revision itself
leaves alone**, because the build before it wrote the row being judged. When unsure, the
reach is everything.

**A row from before the column existed reads as "nobody knows" rather than as any revision.** Nothing
recorded what produced it, and stamping it with a number now would claim it agrees with this build.
The cost is one reading of the corpus, once, the next time a scan runs. The Scan page says how
many songs are waiting, because a stale corpus and a current one otherwise look exactly alike.

**The page and the flag are the same operation over deliberately different sets.** *Re-analyze
everything* re-reads every file there is and regroups the near-duplicates.
`km-package-builder <folder> --reanalyze` reads **one copy of each song** — a bit over half the
files — because the copies are byte-identical by construction. The id *is* the hash, so a second
read produces the same answer for the same row. After a revision moves, that is the cheaper way to
bring a whole corpus up to date. It is also the only one that runs in a terminal and ends with a
status a script can read.

**The flag is for a terminal, and the windowed executable refuses it.** A GUI-subsystem image hands
the prompt back before the work starts. A run typed into it reports into a prompt that has moved on
and ends with nothing to wait for. `--backup` shares the subsystem and loses nothing, being over in a
second; hours of it is another matter. The refusal names the way out, which is
`km-package-builder-console`. What happens is that somebody types the right thing into the
wrong one of two executables.

**What a scoped run does not do**, and the page's own button does:

- it reads one copy, so a *copy* edited on disk is not noticed until a real scan
- it does not look for near-duplicates, so a grouping stands on the fingerprints it was made from
- it forgets nothing that has gone
- it stamps no `last_scan`, because a corner of a corpus is not a reading of the corpus

## Assigning tags in bulk

**The tag control adds or removes one tag; there is no *replace*.** It is otherwise the language
control exactly: the same steps, the same scope select over ticked rows or the whole filter, the same
ticked default. What a tag is forces the one difference. A song has one language,
so writing it is a complete statement. A song has many tags, so a *set* would silently destroy
tagging elsewhere, in one click, over a filter spanning every row in the corpus. Both acts that
remain are additive judgements about a set somebody has already described,
which is what makes the filter-wide half defensible at all.

**There is no *only where nobody has said* box either.** For a language, that box narrows the
*write* rather than the list. That is exactly what the bar's own `language=unset` cannot do without
taking the rows being looked at off the page. A tag has no such state: every song starts with none,
and most end that way. So the equivalent narrowing is the bar's own tag filter, and that one leaves
the rows in front of you. It selects songs that have a tag rather than songs that lack one.

**The confirmation shows the slug, not what was typed.** `Rock & Roll` is stored as `rock-roll`, and
this is the one place somebody finds that out before the write rather than after it. That is worth
a step the language control has no need of, because a language picker cannot be typed into.

**A vocabulary of suggested tags lives with the person, not with the corpus.** Nothing detects a tag,
so a corpus nobody has tagged offers an empty picker. The first tag then has to be invented with no
hint about what a good one looks like. `pop`, `rock` and `classic-rock` are that hint, and they sit
in the curator's own config directory beside `recent.json` rather than in the workspace database.
What a good tag looks like follows a person from one folder to the next, while a machine's address
does not. Three, not thirty: this is a hint about the *shape* of a tag, not a taxonomy.

**A suggestion never reaches a package**, and that is a property rather than a promise. The build
reads `song_tags`, so a word nobody has put on a song has no row to be read from. Because a catalog
is built from packages, it cannot reach a machine or a phone's mirror either. They are drawn dashed
and dimmed against the tags in use, because the two are different claims. *Songs are filed under
this* differs from *this is the sort of word to use*. Confusing them would have somebody filter by a
suggestion and conclude their corpus is empty.

## Where a curation action says what it did

**A toast for what finished, and the message slot for what was refused.** A message swapped into
`#action-result` sits at the top of a page, because a refusal can run to a paragraph. One printed
under a hundred rows is off the screen. The same hole appears from the other end: pressing a
button near the bottom puts the answer a screen and a half above the cursor. The browse list showed it
first, and every page that grew a second panel has it since. The lists a package draws on sit below
three other cards, and *3 come out* printed above them is printed where nobody is looking.

**So a write that succeeded toasts, and a write that was refused keeps the slot.** What succeeded is
news about something that is over, which is what a toast is for. A refusal names a remedy somebody has
to act on — re-flow this package, take the working-list mark off that list, choose another name. So
a sentence that fades after eight seconds is a sentence that has to be produced again to be read.

**A song's own page follows the same rule for a play and a saved correction list.** Its Test-play
button is at the top, and its Save corrections button is under a channel table. Both answers are
news about something that is over.

**A question is not a message and never fades.** The sync confirmation and the five bulk confirmations
are forms with an Ok and a Cancel. They stay in the slot beside the button that raised them.

**A report keeps the slot too**, by the same rule read the other way. Opening a built package can
answer with ten songs it could not match. A build says where it wrote and what it re-encoded.
Those are read, compared and acted on rather than noted.

**The same question splits the header's two buttons, and they answer it differently.** *Open in
browser* toasts: a window opened somewhere else is over the moment it is said. Its slot is inside
the header, where a sentence naming an address widens the one strip every page is measured against.
*Quit* keeps the message, because *Stopping. You can close this window.* is a permanent state, not
news about something finished, and a claim must not fade after eight seconds onto a header that has
gone quiet.

**The half of `#toasts` that must stay client-only is untouched**: `/songs/rows` answers a database
error with a 500 and a body htmx will not swap. A 200 there would paint the error over the
hundred rows being read, and the browser is the only thing that can report it. What the server half
adds is the opposite case, an action that *succeeded*.

**It travels as an out-of-band swap in the body and not as an `HX-Trigger` header.** The reason is
encoding rather than taste: `XMLHttpRequest.getResponseHeader` decodes a header as ISO-8859-1. Every
message here can carry a Portuguese title or a path out of the corpus. A body, by contrast, goes
through askama's escaping and arrives as the bytes that were written. Which answer a route gives is
the tool's existing `?as=` idiom; `ui.js` *manages* the tray rather than filling it. A toast
from either source fades, dismisses and expires identically.

**The corner is the top right, newest at the top, four at a time.** The eye goes to the top of a page
after pressing a button on it, and the sticky header is a measurement, not an obstacle.
`ui.js` writes its height into `--header-height`, and the tray starts below it at every width. Newest
at the top follows from the insertion order both halves use and means the message just produced is
nearest the corner. The cap serves the bulk actions, which say four things about a hundred rows
faster than anyone reads them. Past the cap the oldest goes at once rather than burying the
newest, and each toast leaves on its own eight-second timer.

## A count that did not happen says why, and is never a subtraction

**Adding songs to a package has three outcomes, and they are counted apart.** They are numbered in,
already there, or left out because the numbering reached the last slot a singer can dial. Taking the
number added from the number asked for reports all three as *already in it*, which is a true
sentence about the wrong subject. It sends somebody to look at a package that is doing exactly what
it should. The thing actually in the way goes unsaid. So `Added` carries a count per outcome and the
sentence is built from them.

**The refusal names the remedy that fits, and there are two.** A hand add appends to a package's last
volume. A volume whose numbers ran to the end from a high first number is re-flowed, which moves
every song down and frees the numbers above. A volume holding all 999 is full, and offering it Re-flow
would send somebody at a wall. What grows a package past it is sourcing it from favorites, whose sync
starts the next volume. The two are told apart by how many numbers are in use, not by how the ceiling
was reached.

**A batch that placed nothing reads as a failure**, because a green line saying zero is read as
nothing having been ticked.

## What the list of packages shows, and what it does not

**The name is the first column and the link, and the id is not a column at all.** An id is generated
and sixteen hexadecimal characters, so a column of them is the widest thing in the table. It is also
the least like anything somebody is looking for, sitting where the eye lands first. It is on the
package's own page beside its name, which is where it is wanted. Reading an id off is done once, to
match a bank or a built file to the package it came from.

This is the rule [`What the tool calls the package it writes`](#what-the-tool-calls-the-package-it-writes)
already states about file names, in the one place left that still led with the id.

## A package's first number is a slot, and one that is not is refused

**The box offers 1 and takes 1 to 999**, which is what a song's number inside a package is. The
machine adds the thousands that say which package it is, and it assigns them itself from the package's
id. Nothing typed there can collide with a package already installed, so there is no range for a
curator to keep clear of. A box that invited them to pick one had them start a package at a number
no song in it could carry.

**A typed number outside that is refused in words rather than lowered in silence.** The two are not
the same package: a first number quietly moved to the last slot makes one that takes a single song.
It says nothing about why. A number read out of a built `.kmpkg` is still clamped, because an import
has no form to be answered. Refusing it would throw away the songs to save the number.

## A package can be the songs in some favorites

**A package names the favorites it draws on, and its songs are what those lists hold.** The deciding
happens in the lists — `Brasil Axé`, `Brasil Samba`, a pass through a folder putting what it settles
somewhere. A volume made out of them is the same judgment read out. Copying it once and never
hearing about the list again is what this replaces. A song starred afterwards had to be put in the
package by hand. A song taken out of the list stayed in the package for good.

**Naming a list is the whole of the setting, and there is no second box.** A package with at least
one source is *sourced*; taking the last one away makes it an ordinary package again. A separate flag
could be off while the sources were named. That would leave a package that accepted songs one at a
time and destroyed them at the next sync. One control would say the opposite of another, about the
same package, on the same page.

**The lists a package reads are a table, and the rest are a picker.** A curated corpus carries lists
in the dozens, and a box per favorite wraps into a paragraph. There, a name, the count beside it and
its mark each land on a different line from the box they belong to. So reading the paragraph, not
looking, answers *which of these does this package read*. Four sources in four rows answer it
at a glance, each with the Remove that is the only way to take one away. Adding a fifth is a name
picked out of a select, and the picker offers what is not already read, so no list appears twice.

**So a sourced package is absent from every control that adds a song to a package**, rather than
present and refused. A song put in one goes out on the next sync, which is a failure that
arrives days later as songs that will not stay. The two selects that offer packages leave it out.
The route they post to refuses it by name for the page that was open before the list was ticked.
Taking a song out by hand is still offered, because the next sync is what puts it back. A button
greyed out to imply a rule it cannot enforce says less than one that works.

**The union is of distinct recordings.** A song filed under two of the lists is one entry: a curator
has said two things about it and asked for one song. A song merged into another arrives as the one it
turned out to be, which is the rule every query in the tool already follows. A star put on a file
before anybody noticed it was a second copy goes on naming the row that lost. Two *versions* of one
recording both starred are two entries, kept as a hand add keeps them: `duplicate_of` is a guess,
`merged_into` is somebody's word. Tidy on the Favorites page is what drops the second copies.

**A working list is offered to no package.** Such a list holds songs somebody set aside to decide
about later — see [`A favorite can be a working list`](#a-favorite-can-be-a-working-list). A
volume that followed one would ship what nobody has decided about, growing every time the list was
added to. It is absent from the picker and from the offer a new package makes, and the statement that
writes a source is what refuses one. So a list set aside between the click and the write is refused
too.

**This is narrower than what a working list's songs may do**, which is go into a package like any
other list's. A curator adding them is a person choosing each one. What is refused is a *standing*
arrangement to take whatever such a list comes to hold.

**One already read stays, and is marked.** The flag is set on the Favorites page, so a list can become
a working one after a package began reading it. A row that disappeared at that moment would leave the
package drawing on a list its own page did not show. There would be no way to take it away. The sync
goes on reading it, because a source that silently contributed nothing would be worse than one
somebody can see and remove.

**Deleting a list takes it out of the packages that read it, and takes no song with it.** That is the
answer a saved filter naming a list that has gone already gets, and it is silent. So the Delete
button names the packages first, because the cost lands at the next sync rather than at the click.

**Making a package of a filter offers to source it, when the filter is one list and nothing else.**
Narrowing the bar to `Brasil Axé` and asking for a package of every matching song has already said
what the package is. So the box is there, and it is ticked. Unticking it is one click, and the package
is an ordinary one. A filter naming a working list gets no box, by the rule above.

**A filter carrying anything more gets no box at all**, rather than an unticked one. `Brasil Axé` and
Portuguese makes a package the list alone would not. A package claiming that pair as its source
would pour every other song in the list into itself the first time anybody pressed Sync. The page a
curator is on, the order the rows are in and how many of them there are do not count as narrowing.
Neither does collapsing a cluster to one row, which decides which rows are drawn rather than which
songs a list holds.

**The box is read again when it is answered.** A confirmation can sit on the screen while the bar
behind it is narrowed further. A box drawn over one filter must not be obeyed over another.

## What a sync keeps, and what it hands out

**A song in both the package and its lists keeps the volume and the number it had.** A songbook
printed from a package goes on being true for every song still in it. That is the promise the whole
arrangement rests on, and the reason a sync is a thing anybody can press twice.

**A song arriving takes the lowest free number**, in the first volume that has one, from that volume's
first number upward. It only appends past the highest when the holes are used up. Numbering from the
highest is what a hand add rightly does, because the room it has left has already been said out loud.
For a sync, though, that would spend a volume's 999 slots on the songs a list has held and lost. A
list edited a few hundred times would run out with forty songs in it.

**A song leaving does not make a hole.** Its number stays held for it, so a songbook printed before the
sync stays true. The holes a sync fills are the ones a person made, by releasing a hold or by moving a
song. See
[`A sync holds the number of a song that leaves`](#a-sync-holds-the-number-of-a-song-that-leaves).

**A union larger than every volume starts another volume.** A sync places every song its lists name,
and the confirmation says how many volumes it would start before anything is written. See
[`A package holds volumes`](#a-package-holds-volumes).

**Re-flow still means what it meant, inside one volume.** It moves every song of that volume down from
its first number and closes the holes. On a volume that has churned, that is exactly what somebody
wants. The two do not fight, because a re-flow moves everyone and adds nobody, where a sync adds,
removes and moves nobody. A held number is not a hole, so a re-flow flows the songs around it.

## A sync holds the number of a song that leaves

**A song a sync takes out leaves its number held for it.** A printed songbook still lists the song at
that number. A singer who dials it would hear a different song if the number went to the next arrival.
So no sync, hand add or re-flow gives a held number to another song.

**The hold names the song it waits for.** The Songs tab draws it as a row at its number, with the
title and the performer the song had when it left. The title links to the song while the corpus still
knows it. The row tells the curator to put another version of the same song there.

**A person fills a hold, and only by moving a song the package already holds.** The row offers a button
for each song of the package that the grouping pass files with the missing one. It also takes a volume
and a number, because the grouping pass does not find every copy. The song may come from any volume,
and this is the one move between volumes. The number it leaves is an ordinary free number, because only
a sync makes a hold. Typing the held number into a song's own number box fills the hold as well.

**A song that comes back to the lists takes back its held number.** Its songbook entry is then true
again. A merge counts: a list holding the song it was merged into brings that song back to the number.

**Release gives a hold up.** The number becomes free, and the next sync can hand it to another song.
A hold spends a number until somebody fills or releases it. A list edited many times therefore fills a
volume with holds, and Release is the answer to that. A replacement at a held number is refused, because
the hold names what belongs there.

**Re-flow keeps every hold where it is**, and flows the songs around them.
## A song in a package can be replaced at its number

**Another song takes the number, in the same volume, and the song that held it leaves the package.**
A better file of a song already packaged turns up often, and the number is what a songbook prints and
a singer dials. Remove and Add would give the new song the number after the highest.

**Any song can take the place of any song.** The new one does not have to be a version of the old one,
because the grouping pass does not find every copy. A curator may also choose a different
recording on purpose. The control is on the new song's page, with a number and a volume. It asks
first and names the song that would leave, because a number typed from memory is the likeliest
mistake.

**It is refused** when no song has that number, or when a sync holds the number. It is also refused
when the song already holds the number, is already elsewhere in the package, or was merged into
another. A package holds a song once, and a merged song is shown as the one it was merged into.

**In a package that follows favorites, the lists change as well.** A sync cannot tell that the new song
stands for the old one. On its own it would take the old song out, hold its number, and give the new
one a different number. So the new song takes the old one's place in each list the package follows.
That includes a list holding a song merged into the old one, so the next sync has nothing to move.
A list the package does not follow is left alone, and the confirmation names the lists that change.

## A package holds volumes

**A package is what a curator names, and a volume is what a build writes.** A package holds one volume
or more, and each volume is a `.kmpkg` with its own id and its own version. Each volume also has its
own first number and its own 999 numbers. The name, the publisher, the default language, the
raise-the-version box and the favorites a package reads belong to the package. Everything that
differs between two files of it belongs to a volume.

**Only a sync starts a volume.** A package sourced from favorites whose lists outgrow every volume it
has gains as many volumes as the overflow needs, numbered on from the last. A hand add appends to the
last volume and stops at 999, and so does a package made from a filter. **The 999 cap keeps its
purpose that way**: it discourages pointing the builder at a corpus and packaging all of it. A
volume is filled from lists a person curated one song at a time, rather than from whatever a filter
matched.

**A song is in a package once, in one volume, and stays there.** A sync never moves a song between
volumes, so a volume's printed book stays true for every song still in it. A song leaving keeps its
number held, and the next song arriving takes the lowest free number in the first volume that has one.

**Volume 1's id is the package's id**: every package from before volumes became a package of
one volume without a machine noticing. Its bank, which comes from the id, stayed where it was, and a
rebuild still replaces the file installed. A later volume takes a generated id, so each volume lands
in its own thousand on every machine by
[`A package's bank comes from its id`](packaging.md#a-packages-bank-comes-from-its-id-and-from-nothing-else).
**The volumes are not given consecutive banks.** The song book sorts by artist across the whole
machine, so a singer never reads where one volume ends, gaining nothing from neighboring thousands.
A bank that followed anything but the id would be right on one machine and a guess on
the next.

**An emptied volume is kept.** Its id is what a machine banked, and a volume removed and started again
later would land in a different thousand. The next songs a sync places fill it first, and a build of
an empty volume writes nothing.

**One volume looks like no volume**: its file and its manifest carry the package's name, and the
package page draws no volume strip. With two or more, every volume is numbered alike — `Brasil Axé 1`,
`Brasil Axé 2` — so the first volume's name changes at its next build. **How the number is written is
the package's to choose**: a volume name field over the tabs holds a format where `{n}` is the
number. So `vol{n}` names them `Brasil Axé vol1` and `Brasil Axé vol2`. The default is `vol{n}`,
because a bare number lands beside the version in a file name and `brasil-2-1.0.0` reads as one run of
digits. A format without `{n}` is refused, because every volume would then take one name and one file.

**A package can number its only volume.** A tick box under the volume name field names a package of
one volume `Brasil Axé vol1` from its first build. It is for a set its curator knows will outgrow 999
songs. The first file then keeps its name when the second volume starts, and nothing built from it is
renamed. The box is off by default, because most packages never grow a second volume. It changes the
name and nothing else, so the page still draws no volume strip until there are two.

**The Songs tab picks a volume above its song list, and the Build tab picks its own.** On Songs a row
of tab-like links sits directly over the songs, and the first number and the songs follow it.
The link is in the address, so a reload keeps it. The Build tab has its volume picker, because which
file gets built is chosen there, and the version, file names and install button follow it.
**Picking a volume on Songs keeps the page where it was**: the link swaps only the volume's part, so
somebody halfway down keeps their place. The links are drawn as filled tabs, because underlined
words under the page's own tabs would blend in and go missed.

**Re-flow is under the song list, behind a closed Advanced summary.** One press renumbers a whole
volume and asks nothing, and a curator seldom wants it. Beside the first number, under the tabs, it
sits where a press meant for something else lands. It still acts on the volume picked above it.

**Build every volume writes each volume that holds a song under its default name**, into the one
folder. The Build form's two tick boxes apply to every file. Default names, because there is
one name box and the run writes several files. The volume number and the version in each name keep
them apart. A volume that refuses — a song with no language, say — is reported and the next volume
is still built.

**What belongs to the whole package is a form over the tabs**, shown whichever tab is open. It lists
the name, the publisher, the default language and the id. **The id appears there and nowhere else on
the page.** It matters only when a built file or a bank has to be matched to a package. Drawn beside
a heading or a picker, it is sixteen characters in front of somebody reading for something else.

**A built volume says which package it belongs to**, through the manifest's `volume` key — see
[`A package file can say which set it is a volume of`](packaging.md#a-package-file-can-say-which-set-it-is-a-volume-of).
Importing it joins that package as that volume. An id that disagrees with the volume already there is
refused. Two files claiming to be one volume are two packages that must not be merged by
accident.

## A sync is pressed, and it asks first

**Nothing syncs on its own.** A build packages what the package holds, and saving the sources writes
the sources. Pointing a package at a different list is a decision. What that would do to its songs is
a question, not a consequence of answering a different one.

**It counts, shows the lists as chips, and asks**, which is
[`Acting on a whole filter`](#acting-on-a-whole-filter)'s discipline over a set that is not a filter.
What earns the question is the removal: the ticked half of *add to a package* is the one control there
that asks nothing. What excuses it is that its set is the rows in front of somebody. A sync's set
is a list on another page, and the rows it would take out are not on the screen at all. The count
names what goes in, what comes out, what stays, and what would not fit.

**A package that already agrees with its lists gets a sentence, not a question.** Pressing Sync on one
is an ordinary thing to do, and the answer is yes. A confirmation offering to change nothing is a
dialog nobody reads.

**A package sourced from nothing is refused rather than emptied.** Pressing Sync on one nobody has
given a list to means nothing, and of the two readings available the other one costs a package.

**The member table comes back with the sentence.** Every other write on that page moves one row or one
number, and the person who pressed it is looking at what changed. A sync adds and removes many at
once, so a list left as it was would contradict the count beside it.

## The curation actions are tabs

**One strip, one tab per action, and nothing stacked.** Under the Songs page's filter bar sit ten
things that can be done to what it selected:

- set a language
- add or remove a tag
- make a package of the filter
- add songs to a package that exists
- file them into a favorite
- take their titles from their file names
- fix the capitals in the names they have
- take the artist out of the title it was filed inside
- recalculate their suitability
- number them for the order to play them in

Stacked down the page they read top to bottom as a sequence when what they are is alternatives.
Worse, the two that act on the ticked rows sat among the two that act on the whole filter with
nothing to tell them apart. That is the one distinction
[`Acting on a whole filter`](#acting-on-a-whole-filter) exists to keep sharp.

**One action per form.** *Add to a package*, *file into a favorite* and *Title from file name* would
otherwise share `#selection`, divided by two dim `|` characters. The seam shows in the markup:
the File button has to `hx-include="#rows, #selection"` to reach a select two controls to its left.
One action per tab forces the split, and each form posts what it holds plus the ticks. No button
reaches sideways for anything.

**A tab is one subject, and several forms under one subject are one tab.** *Make a package of every
matching song* and *add to one that exists* are one subject, divided by whether the package exists
yet. Each offers the scopes [`Acting on a whole filter`](#acting-on-a-whole-filter) names. *Title from
file name*, *Fix the capitals* and *Artist from the title* are one subject in the three ways a name is
wrong:

- there is no title worth keeping
- there is one and it is shouting
- there is one and it holds the artist as well

Neither group is more than one tab. A curator looking at a bad name should not have to know which of
them it is before choosing where to look.

**Nothing auto-hides.** A CSS rule dropping `#selection` whenever nothing was ticked would move the
rest of the page under the cursor as rows are ticked and unticked. It would also take the one bar
that says what ticking is *for* off the page where nobody has ticked anything yet. The empty case is
not silent either. Every one of these answers it with *nothing is ticked*, which is a toast in the
corner rather than a button that does nothing.

**Radios and `~`, not `:has()` and not a script.** The obvious markup is the alphabet strip's, with
each radio inside its own label in the strip. A selector then has to reach out of the strip and
sideways to a panel, which needs `:has()`. That is the wrong failure polarity here. The rule hiding
a panel is unconditional. A browser that could not evaluate the `:has()` would show *no* panel
and lose the whole of the page's curation.

Flat siblings, every radio before every panel, need only
`:checked ~`. A stylesheet that does not load at all leaves every form on the page, which is
still a usable page. The radios sit outside every form for the reason the alphabet strip sits inside
one: `#filters` carries `hx-trigger="change"`. A radio in it would fire a filter request per tab
click. A native radio group is also arrow-key navigable, which is what the tab pattern asks for. It
is also what a row of buttons would have had to be given.

**Which tab is open is not remembered**, for the reason
[`Ticking a whole page of songs`](#ticking-a-whole-page-of-songs) gives about the box in the table
head: it means *this page*. No panel is inside `#rows`, so no swap disturbs it. A fresh load
starts at Language, which is where the work is on a corpus nobody has classified.

**The Lyrics page offers one of these actions and therefore draws no strip.** A lyric search answers
*which song goes like this?*, and the answer is set aside for the pass that decides. Filing the hits
into a favorite is the action that belongs there. The rest are absent rather than empty:

- several read a filter bar that page does not have
- the three that correct a name are judgments made while looking at one, which that list does not show
- a package is built from a filter, which is the same thing missing

**One action is not a tab.** A strip of one is a
label over the only panel there is, and it asks somebody to choose before they are offered anything.

**A tab is drawn whether or not it has anything to offer.** A favorites control wrapped in a
condition is fine for a group inside a bar of other controls, and wrong for a tab. An empty tab
would be a panel behind a label promising something. It says what is missing and where to fix it,
which is what the package select already does.

## Putting capitals back into a name that has none

**A corpus arrives shouting, and one button on the ticked rows answers it.** `CORCOVADO` by
`TOM JOBIM` is not one file's mistake. A title meta event written in capitals is the ordinary output
of the sequencers that made this corpus. The stem *Title from file name* writes is the same shape
again. A page of ten thousand capitalized rows is harder to read than the same rows in mixed case.
Correcting them one edit box at a time is not a pass anybody finishes.

**Case a person typed is evidence of a decision. A typed name holding both a capital and a small
letter is left exactly as it is.** `d'Angelo`, `McCartney` and `Tom Jobim` are answers, and no rule
this size can tell one from a mistake. What is rewritten is a typed name that is entirely one case —
which covers the whispering half, `garota de ipanema`, by the same test.

**Case the file carried is not a decision, so a name from the file is rewritten whatever case it
has.** `Garota De Ipanema` and `Tom jobim` in a title meta event are a sequencer's output, the same
defect as `CORCOVADO`. A guard that skipped them would leave the button doing nothing on the rows
nobody has touched yet. Its own source judges each field, so a typed artist keeps its case beside
a title from the file. The cost is a file name that was right in an unusual case: `McCartney` from the
file comes back `Mccartney`. One edit puts it back.

**An acronym with no case of its own is recased with everything else. That is the limit of the
guard, not a gap in it.** `AC/DC` among `CORCOVADO` is the same nine bits to any rule here, so
it comes out `Ac/Dc`. The guard protects a name somebody has cased; an all-capital acronym is a name
nobody has.

**The button promises a first pass and says so on the page.** A rule that gets
every roman numeral and every surname right does not exist. The alternative to an imperfect pass
is a corpus left shouting.

**One list of small words, English and Portuguese together, and not one list per language.** Almost no
song in a real corpus carries a language. Curation sets the column, and this button runs long before
that pass finishes. A list chosen by the language column would be inert on the rows
that need it most.

The cost is the few words the two languages share and disagree about: `do` and `no`
are Portuguese articles and English verbs. So *I Do Love You* comes back as *I do Love You*. The
corpus is majority Brazilian, which is the side to be wrong on, and a row it gets wrong is one edit.

**The word that opens a phrase and the word that closes one are capitalized whatever the list says.**
*The Dark Side of the Moon* keeps its leading `The`. *What Are You Waiting For* does not trail off.
A phrase in brackets or after a spaced dash begins the way the whole name does — *Chega de Saudade
(Ao Vivo)*, not *(ao Vivo)*. A dash without space is inside a word and starts nothing.

**Ticked rows, not the whole filter**, for the reason
[`Acting on a whole filter`](#acting-on-a-whole-filter) gives and *Title from file name* already
follows. Whether a name is shouting or spelled the way somebody wanted is a judgment about that file,
made by looking at it.

**It writes `title` and `artist` and never the detected pair**, which is the split the whole `det_`
half of that table exists for. What the file said stays answerable. A row showing nothing but its file
name is skipped rather than given a title. Following `eff_title`'s fallback to the stem would quietly
make this the file-name button as well. That is two actions in one press, and only one of them asked
for. The button beside it is the one that gives a nameless row a name.

**A song that already had capitals is not a failure and is not reported as one.** The count short of
the number ticked is said in the same sentence as the count fixed. A toast reading *fixed 1 of
3* with no explanation reads as two things having gone wrong.

## Taking the artist out of the title it was filed inside

**A corpus files one song's two fields in one string, and one button on the ticked rows takes them
apart.** `Bob Dylan-A Hard Rain's A-Gonna Fall` and `Bob Marley - Natural Mystic 1982` are a title
meta event and a file name from the same world. Each leaves the artist column empty while the
artist sits in front of a curator inside the title. An MP3+G stem is the same shape by measurement
rather than by accident, which is what
[`Where an MP3+G song's title and artist come from`](song-sources.md#where-an-mp3g-songs-title-and-artist-come-from)
records. Retyping both fields a row at a time is not a pass anybody finishes on a corpus this size.

**The first `-` is the seam, and only the first.** A title holds dashes of its own — `A Hard Rain's
A-Gonna Fall`, `Re-Recording`. So a rule taking the last dash, or every dash, cuts inside the song's
name. The artist comes first in every spelling of this shape the corpus has. That is what makes the
first dash the seam rather than a guess. Both halves are trimmed, so the spaced and the unspaced
spelling of one defect answer the same way.

**Only the ASCII hyphen.** An en dash and an em dash are typography. A corpus written by
sequencers on code pages does not carry them where it carries this defect. A rule reaching for all
three would cut a name that used one deliberately.

**Only a song with nothing in its artist cell** — the one reading of that column this action
does not share with the browse list. An artist recorded as the empty string counts as blank here
where the list sorts it as a value. `''` is what *Title from file name* leaves behind, and those rows
are exactly the ones still carrying an artist inside the title. So the follow-up press is the point
rather than an edge case. A song that names an artist has had this judgment made about it, whether
the name came from the file or from a person.

**It reads the name on the screen, file name and all**, which is `eff_title`'s three steps. A row
whose only name is its stem splits too, so `bob_marley-zimbabwe` becomes an artist and a title in one
press. That is the opposite answer to *Fix the capitals* beside it. What separates them is what
the write puts in: recasing a stem row would be the file-name button's work done a second time.
Splitting one fills the artist column that button never fills.

**A split needs both halves.** `-Foo` and `Foo-` hold no artist to take out, so they are left exactly
as they are rather than given an empty artist. That is a decision, and not one a rule this size is
entitled to make.

**It writes `title` and `artist` and never the detected pair**, as the two buttons beside it do.
So what the file said stays answerable, and the row lights the `ed` tag.

**Ticked rows, not the whole filter**, for the reason
[`Acting on a whole filter`](#acting-on-a-whole-filter) gives. Whether the words before a dash are a
performer or part of a song's name is a judgment about that file. That judgment comes from looking at
it. `Blue-Eyed
Boy` is what the filter-wide version would cost.

**A row there was nothing to take out of is not a failure.** The count short of the number ticked
is said in the same sentence as the count split, naming both reasons a row was passed over. The rule
is the capitals button's and for the same reason: a bare short count reads as something having gone
wrong.

## A song's own page is four tabs

**Four subjects, one tab each: what this song is, what it is on disk, where it has been put, and what
it says.** The page holds nine boxes:

- the editable facts
- the file analysis
- the copies on disk
- the other files that look like this recording
- tags
- favorites
- packages
- the lyrics
- the raw text events

Side by side down a page they read as a sequence. What they are is four things somebody comes here
for one at a time. *Details* holds the facts and the analysis, because a suitability of 4 is read
while deciding whether the title is worth typing.

*Files* holds the copies and the other versions, which are the same question at two distances. How
many files are these exact bytes, and how many files are this same song saved differently. *Filing*
holds tags, favorites and packages, which are three ways of putting a song somewhere. *Lyrics* holds
the words and the events they were read out of.

**The `.curate` pattern with a prefix of its own**, and the reasoning in
[`The curation actions are tabs`](#the-curation-actions-are-tabs) reaches here unchanged.
Hidden radios, flat siblings, `:checked ~`, no `:has()` and no script. A stylesheet that does
not load leaves all four panes on the page rather than none. The prefix is not decoration: the Songs
page already spells `curate-tags` and `curate-favorites`, which are exactly the two names this set
would want.

**A pane holds cards and is not one.** The boxes are bordered cards, and the class that draws them is
the class `.curate` hides. So a pane draws nothing of its own and lays its cards out in the same
grid that put two side by side before. The strip then hangs over cards rather than over a box
holding cards. A tab of two cards is as wide as a tab of one.

**Details opens, and which tab is open is not remembered**, for the reason the curation strip gives.
A song page is arrived at to read what a song is, and the work of a page is what its first tab should
be.

**What a button answers with lands above the strip.** Nearly every control here targets one slot.
A slot inside a pane is a button that appears to do nothing whenever another tab is open. The two
banners are outside for the stronger version of the same rule. One is a song merged into another;
the other is a song a suggestion pass hid. A page that is hidden from browsing has to say so however
it was left.

**The lyrics load with the page rather than when their tab is opened.** A `display: none` element is
never *revealed* by a CSS rule changing. So deferring them would mean the words arriving only for
somebody who opened the tab and then scrolled. The raw text events stay a button, as they were. That
one is a click because the text can run to thousands of lines, not because the tab is shut.

## Settings is five tabs

**Five subjects, one tab each: the machine, the backup, this program's language, the tags it
suggests, and what this folder holds.** They share nothing. Stacked down a page they read as a
sequence to work through. What they are is five places somebody comes to one at a time. The
one wanted most often — which machine Play and Install reach — sat above four panels it has nothing
to do with.

**The same `.songtabs` the song page uses, and the element carries both names.** The layout, the
hidden radios and the rule that hides a pane are one arrangement written once. What cannot be shared
is the arm per tab, because a selector cannot ask which radio is checked without naming it. `.pane`
rather than `.panel` for that page's own reason. The boxes here are already the bordered card, and a
rule hiding `.panel` would take every one of them off the page.

**Machine opens**, being the setting most often come here for and the one a folder pointed at nothing
needs first.

**What a button answers with stays above the strip.** Four of the five panels swap one slot.
A slot inside a pane is a button that appears to do nothing whenever another tab is open. Choosing a
language is the fifth and aims at nothing, because it redraws the document. The redraw reopens
Machine, which is the *which tab is open is not remembered* rule arriving for free.

**This folder names the size of the discard pile, and it is the only place that does.** The browse
list shows what has been thrown away only to somebody who asks for it. A corpus holding a hundred
discarded songs therefore reads exactly like one holding none. The pile is a list somebody can still
act on, and one whose size is nowhere is one nobody goes back to.

**That row is drawn at nought as well**, unlike the header's failure badge. The badge is a call to go
and look, so a nought would be a permanent fixture saying nothing. This is a page somebody opened to
read facts about a folder, and there a nought is the answer.

## What a browse list shows without being asked

**Not the file names.** A great many of this corpus's files carry no title at all and many more carry
`UNTITLED`, `Karaoke` or the name of whoever sequenced it. So the name on disk is routinely the only
thing on the row that identifies the song. **None of that is what the default decides.** What it
decides is how wide a row is.

The chip carries a whole base name with its extension, on every line,
and `td.actions` is `width: 1%`. So the space comes out of Title and Artist until the table stops
fitting, and `.scroll` starts scrolling sideways. That, in the tool's own 1500-pixel window, is the
ordinary case rather than the narrow one. **A box turned on when it is wanted costs one click; a table
that overflows costs the two columns somebody was reading.**

**An unticked checkbox sends nothing, which is byte-for-byte what a page that never had one sends.**
Off by default is what collapses those two meanings into one, so `filenames()` is one line and no
marker field is needed to separate them. The obvious alternative, a hidden `filename=0` beside the
box, is a 400. Serde's derived visitor answers a repeated key with `duplicate_field`, so *ticking* the
box would fail. That `duplicate_field` rule is load-bearing elsewhere and is recorded in
`Acting on a whole filter`.

## The order of a browse row, and the two marks on it that carry a color

**Artist first, then title.** A person walks a corpus by performer, asking *what else did they do?*
That is the question the artist link answers, and the one that fills a package. So the artist column
is read first, and it must not be the one squeezed first when the table outgrows its width. That is
the ordinary case rather than the narrow one.

**The sort does not follow it, and stays on the title.** The column order says what the page is
*for*; the sort says what order the answers come in. A page of fifty is a batch of work somebody
finishes. Finishing it alphabetically by title makes the same folder open on the same page tomorrow.
*Artist* is one click away in the bar for the pass where it is the right order.

The performer is the second key rather than the first. That draws the four *Goodbye*s by one
performer together, instead of scattering them through the others. The rule and the term that puts
an unattributed song last are `Within one title, by performer` in [`songs.md`](songs.md).

**A filed song's star is gold and not merely filled.** ★ against ☆ at row height is a few pixels of
interior. Being in a favorite is the one thing on a row that is scanned for down a column, rather
than read across one. Color is what a glance sees. It gets a token of its own rather than the
warning gold it matches, because the two make opposite claims. One says something is off; the other
says somebody chose this.

**The fill and the color answer different questions.** A
[working list](#a-favorite-can-be-a-working-list) is what separates them. The fill says the song is
in a list, and the color says one of those lists is a filing. A song somebody has set aside to decide
about later is a song still to do, and gold down that column has to mean done.

**The star's chooser opens under the row, not over it.** *Which favorite?* is a question about a
particular song. The artist, the title and the file name on that line are what somebody answers it
from. A chooser standing where the row stood takes the question's subject off the screen at the
moment it is being asked. So the row keeps its line and the chooser gets a second one beneath it.

**The favorites and the two buttons beside them are drawn as different kinds of thing**, because they
are. Each favorite is a place this song could go, and *Create & file* and *Cancel* are what to do
about the chooser. Drawn alike they read as one row of alternatives, one of which quietly does
nothing to any favorite. The two sit at the end behind a rule, and the accent means the action there
as it does everywhere else in this tool. A favorite the song is already in gives up that accent, in
exchange for the star's own gold. That is the claim the row's own star makes, in the color it makes
it in.

**And the star closes what it opened.** The row it is on is still there while the chooser is under
it. So a second click on the same button that does nothing at all would be the surprising behavior,
rather than the safe one.

**A score somebody set is the second, and it is filled rather than colored.** A rating is scanned down
its column the way a star is, and that column is empty in nearly every row of a real corpus. Almost
nothing in a whole corpus has been rated. So a shade of text is too quiet to find, and the cell takes
the fill instead. The suitability one column left stays colored text: that column is
never empty, so a color there separates values rather than finding one.

**The accent, and the star's gold would have been wrong by a hair.** Gold says *somebody chose this*,
which a rating also says. What separates them is that a star is a filing, and a score is a judgment
with a number in it. The accent is what this tool colors a value somebody typed. The `ed` tag in the
title cell makes that same claim about that same kind of fact.

**The row does not say how many packages a song is in, and does not open the file in the OS.** Both
are on the song's own page, which is where somebody has stopped on one song rather than judging a
page of them. Every column and every button on a row is paid for by Title and Artist. Those two are
what the row is read for, and the first to be squeezed when the table outgrows its width. So what
earns a place is what is scanned down a column or pressed on most rows, and neither of these is. A
package's contents are read on the Packages page, a column at a time, which is the question *how many
packages* was standing in for.

## A quality hint is a position on the row, and it is rubbed out rather than kept

**Ticked songs are numbered 1, 2, 3 … in the order most likely to end at the first one.** **So
somebody holding six files of one song knows which to play first.** A corpus holds four to eight
copies of a popular song, and the only way to choose between them is to listen. That is minutes per
song, over a folder of thousands.

The tool already knows most of the answer, and had no way to say it. The browse row shows the
suitability, and
[`Duplicate aggregation`](#duplicate-aggregation) measured that 89% of near-duplicate groups have
their top suitability tied. A number that ties is a number that decides nothing.

**The number is a position and not a rating.** The row already carries the one 0–10 this tool
measures, and the one a person types. A third number claiming to be a third judgment is what this
must not become.
[`Suitability is not called a score`](songs.md#suitability-is-not-called-a-score) is the rule, and a
quantity beside the suitability would break it, whatever it was called. What is shown is where a file
came in a list, which is a fact about the other files ticked, and about nothing else.

**What separates two copies is finer than the 0–10, and every bit of it is already stored.** The
order is:

- the suitability
- its lyrics, sync and arrangement components
- the channel count
- how far the lyric encoding was decoded rather than guessed at
- the copies on disk
- the id

The components lead because the lyrics one is what a singer is looking at. 3 marks where words end;
2 is thin or drawn divided; 1 arrives a line at a time. The last three are what
[`Duplicate aggregation`](#duplicate-aggregation) already chooses a group's representative by.

**Every key is a column a scan measured, and the rating somebody typed is not one of them.** A
rating says how much this song is wanted in a package, rather than which copy of it is the better
file. [`User score`](songs.md#user-score) is where that is argued. So a copy carrying one would
lead its group on an answer to a different question.

**The melody channel is shown and decides nothing**, as it deducts nothing from the suitability.
Whether one could be picked out measures the detector, and how a file was named, rather than how the
song sings. [`Suitability`](songs.md#suitability) is where that is argued. And a hint is no more
entitled to spend it than the number is. The row's own column says whether the machine will be able
to offer the toggle.

**MIDI files only, and the rest are left out and counted.** The tool rates a video song and an
MP3+G song by what they are, rather than by measurement
([`Suitability, for a song that was made to be sung to`](songs.md#suitability-for-a-song-that-was-made-to-be-sung-to)).
Every component behind that number is a fill, so an order over them would be the id tie-break wearing
a badge. Somebody who ticks eight rows and sees six numbers is told why.

**It reads what a scan wrote and never a file.** That is what makes it a button that answers at
once, rather than a second *Recalculate suitability*. That one reads every file and takes a scan's
worth of time. A corpus whose analysis is stale is hinted from the stale numbers, and correcting
those is asked for rather than automatic. The rule [`Suitability`](songs.md#suitability) already
states as much.

**A mark on the row, not a sort of the list.** Reordering the page would lose the place somebody is
browsing, which is most of what
[`The songs page is come back to, not started again`](#the-songs-page-is-come-back-to-not-started-again)
exists to protect. So the filter, the page number and the ticks all stay exactly where they were. A
badge appears in the cell that already holds the tick. That cell means *this row is selected*, which
is what the number is about.

A column of its own would be paid for by Title and Artist, which are
the two things the row is read for. It is also the first to be squeezed
([`The order of a browse row`](#the-order-of-a-browse-row-and-the-two-marks-on-it-that-carry-a-color)),
and it would stand empty on all but the handful of rows anybody ever ticks.

**It lives for as long as the tool is open and reaches no database.** Which four files somebody is
about to play through is a fact like which filter they are looking through, and worth less. See
[`The songs page is come back to, not started again`](#the-songs-page-is-come-back-to-not-started-again)
for why that one is in memory. Being held rather than drawn once, a badge comes back on a page turn
and after a filter change. Being held per run, opening a different folder empties it, because the
ids belong to the corpus that was open.

**The similar-names page offers it too.** That list is the files of one song under its different
spellings, which is the question the hint answers. Its two buttons are the Songs page's, over the
rows ticked in that list. A match draws its number from the same held hint, so narrowing the list
keeps the numbers.

**Clearing is a button and not a timeout.** The numbers are the whole of what this writes, so
undoing it must be as plain as asking for it. A mark that went away on its own would go away while
somebody was still working through it.

## How many rows a browse page holds

**Fifty.** Not a rendering cost — the count is carried in the paging links rather than recomputed.
So a turn is one keyset query at any size. It is about what a page *is*.

A row in this list is not read. It is judged: a title, a suitability, a length, a language, and
five buttons that each do something to that song. A page is therefore a batch of work somebody
finishes. A hundred is more of that than fits on a screen or in a sitting. The pager's *next*
becomes a scroll to the bottom of a page you have given up on. Fifty is about two screens.

**The number is named in one constant and nowhere in the code's prose.** The comments say "a page
of rows", because half of them were making a scale argument that does not depend on the number. The
other half would go quietly wrong the moment it moved.

**And the pages are numbered, because the total is exact.** `Db::song_count` is a plain `COUNT(*)`
over the filter, so *page 37 of 4,805* is a fact rather than an estimate. That is what makes a row
of numbers honest. The pager carries five either side of the page being shown, plus the two ends
where the window does not already reach them. The page being shown is a label, not a button: a
control that reloads the screen is one more thing to press by mistake.

A `« 5` / `5 »` pair is what a pager offers when it does not know how many pages there are. On
thousands of pages of corpus, moving five at a time is not navigation.

**Two things this does not decide.** Whether *next* exists is the database's answer, one row
fetched past the page, and never arithmetic on the total. A stale count must not be able to strand a
reader.

And the count is allowed to lag. A scan writing rows underneath makes it a reading rather
than a fact, which the `~` and the `scanning` tag say. So the window is clamped to reach at least the
page being shown. Otherwise a stale total would draw a *last* button landing behind the page it was
pressed from.

## Ticking a whole page of songs

**A box in the table's head ticks the rows that are drawn, and never the filter.** The alternative
reading, *select everything matching*, is already answered twice on that page. The bulk language set
answers it, and so does *make a package of every matching song*. Both count, show the chips, and ask
before writing anything. A box in a table head that silently meant every song in the corpus would be
the same gesture meaning two very different things.

**It carries no `name` and is never submitted.** The ticked-song actions and the Titles ones
hx-include `#rows` whole. So a named input in that head would arrive in every one of their bodies,
as a field nobody wrote a reader for. It sets the boxes that *are* submitted. Nothing persists it
either — the block is replaced on every swap, which is right for a control that means *this page*.

**The box ticks the rows of its own table**, so the similar-names list carries the same box and it
never reaches rows on another list.

**Between one row and the whole page there is a run, and shift-click is what takes it.** Click a
box, shift-click another, and every box between the two is set. Twelve adjacent songs are otherwise
twelve presses, and the gesture is the one a file manager and a mail client both answer. So a person
who has ticked a box and wants the eleven under it will try it before trying anything else. The
table that is drawn bounds it — the same *this page* scope the box in the head means. The filter
stays out of it, for the reason the first paragraph gives.

**The run takes the state of the box that was shift-clicked**, so one gesture clears a run as readily
as it ticks one. The alternative, a run that always ticks, leaves a mis-aimed press to be undone a
row at a time. That is the work being got rid of. The far end stays where a plain click put it, while
shift-clicks move the near one. So a run is grown and shrunk from one place.

**What the gesture remembers is a box, not a position.** Every page turn and every filter change
throws `#rows` away. A row's own swap throws away one too. So a box that has left the page says so
itself, and the run ends with the block it was in. An index into the list would outlive all three and
reach whatever row had moved into its place.

## What keeps the page you are on

**Anything that leaves the list meaning what it meant; a change to which songs are in it starts again
at the top.** Sorting reorders the same songs and the file-name box only decides whether a chip is
drawn, so page four still means a real page four. Every other control in that bar changes which songs
match, and an offset into the old set points at nothing in the new one. On a narrowing it points past
the end, which is an empty list with a working *previous* button, and reads as the tool being broken.

**A bulk action that redraws the list keeps the page for the plainer reason that it sets no filter at
all.** The three Titles actions write the ticked rows and redraw the table under them. The corpus
being redrawn is the one somebody spent a dozen page turns reaching. What makes that safe on a
*write* is the clamp. Any of them can move a name out of a list narrowed by the search box, by an
artist or by an initial. And a page that has fallen past the end comes back as the last real one,
rather than as nothing.

**The count does not travel with it**, for the other half of the same fact. The two bar controls
carry `total` because neither changes how many songs match. A write to a name can, so the number is
taken again, and the label cannot contradict the rows under it.

**The page number is a fact the server already knows and the page does not.** So it is written onto
`#rows` as `data-offset`/`data-total` on every render, and read back in `static/ui.js`. No route, no
handler, and nothing for `rebuild` to keep in step. A hidden field on the filter bar is the
`duplicate_field` trap for the third time in this crate. The ticked-row buttons hx-include `#rows`
beside `#filters`, so an `offset` on the form would arrive twice in one body.

**A form asks for its page back in its own markup, with `data-keeps-the-page`**, rather than being
named in a list the script keeps. The two are the same behavior, and differ in where a mistake
shows. A list of ids drifts silently when a form is renamed or a third one is added. An attribute
puts the request beside the button making it, and cannot be added anywhere it is not wanted.

## The songs page is come back to, not started again

**The filter survives leaving the page.** So the Songs tab and a song's way back to the list lead
to what somebody was looking through, rather than to the whole corpus.

**A pushed URL is not enough on its own.** `/songs/rows` pushes the filter into the address bar,
which covers a reload, a bookmark and the back button. But a pushed URL exists only in the browser,
and the nav is seven bare hrefs rendered by the server. So going to Packages and coming back would
be a reset. The browser's back button is no help from a song's own page either. It lands on an edit,
rather than on the list, whenever the visit came through a redirect.

**It lives as long as the folder it names is open, and it reaches no file.** The cursor is not in
the corpus database. A `.kmbuild` lives beside the corpus, so that a second machine pointed at the
same drive picks up where the first left off. See [`Curation database`](#curation-database). And
where somebody's cursor happens to be is theirs, rather than the corpus's.

It is not worth a per-user file of its own either, because the same convenience is offered
deliberately one section down. A filter somebody has given a *name* survives the run, can be
several, and says in their own words what it is for. See
[`A filter can be given a name, and then it is not the cursor`](#a-filter-can-be-given-a-name-and-then-it-is-not-the-cursor).

**One folder, one filter.** A filter is *of* a corpus, naming a folder and a favorite out of it. So
closing one or opening another opens on the whole of it. Carried across, it would describe rows that
do not exist. And the chips would explain the empty page in the vocabulary of a corpus nobody is
looking at.

**The page is part of it.** A corpus is judged a page at a time, so where somebody stopped is part
of where they were. Page sixteen of one favorite is a place in a morning's work; getting its first
page instead means finding that place again. The page travels wherever the filter travels. That
includes the nav's Songs tab, and a song's way back to the list. That is what makes looking at a
song, and coming back, land on the row it was opened from.

**A page above the end of the corpus is answered with the last page.** A scan can take rows out
from under somebody who is still paging. An offset past the end is an empty list under a working
*previous* button. A bookmark, a saved filter and a hand-typed address land there the same way.

**An address with no `?` at all is answered with the filter; `/songs?` is answered with the corpus.**
Everything that means *the songs page* and has no filter to state sends a bare `/songs`. That is
the redirect from `/`, the Open page's *back* link, the fragment that follows a folder opening, and
a typed address. Each of them is somebody arriving, which is what the remembered filter is for.

*Clear all* is the other one. It is `/songs?` once the last filter comes off, and somebody who has
just pressed it must not be handed back what they cleared. The single character between those two
addresses is the whole of the distinction, and the reason the filter is not restored by the render
itself.

## A filter can be given a name, and then it is not the cursor

**One remembered filter is a cursor, and a curation pass is not one position.** Classifying the
Portuguese files, finding videos with no stated language, and checking what is still unfiled are
three places somebody moves between across a morning. With one slot, each move means rebuilding
fourteen controls by hand. So a filter can be named, and the names sit in a strip below
the controls that make one.

**Inside the box, and still outside the form.** A named filter narrows the list like every band
above it. But a row floating under the bar's border reads as something else: a stray control
belonging to the page, not the bar. Where it is drawn is not why it sits outside the *form*: it
holds a form, and a browser will not keep forms nested. A text box under a bar that fires
on every change would send a filter request per keystroke. So the box is an element the form sits
in, not the form itself.

**The chips are the band below it, and the last thing in the box.** A chip answers *why am I looking
at these three thousand songs*, which is a question asked with the rows in view. So the answer goes
against them rather than four bands up among the controls that produced it. The strip of names is
about a morning's vocabulary and belongs with the controls; what is narrowing the list right now
belongs with the list. Both are outside the form, and for different reasons: the saved strip because
it holds one, the chips because each is a link. Removing a filter is a page load, so the form that
comes back no longer holds it.

**The chip strip draws nothing when nothing is narrowing the list, and the saved strip draws
always.** A chip strip with no chips has nothing to say, so the bar then ends on the names. The saved
strip still carries the only control that can put a name in it. Neither element leaves the page in
either case, because an out-of-band swap can only replace an element that is there.

**A named one lives in the corpus database, where the cursor does not.** The section above keeps the
cursor in memory for as long as a folder is open. The argument: where somebody happens to be is
theirs, not the corpus's, and is worth no more than the run it was set in. That argument does not
reach a name. *Portuguese, unclassified* is a judgment about how this corpus divides, made once and
worth months, like a favorite or a tag. It is kept where they are kept, so a second machine pointed
at the same drive finds it.

**A new table and no new schema version.** `schema.sql` runs in full on every open, so an added table
is its own migration. Stamping a higher version would make every corpus this build touches
unopenable by a build that does not have the feature. That refusal buys only a strip the older build
would simply not draw.

**The name is the key, and saving under one that exists replaces it after asking.** Adjusting a
filter and saving it again under the same name is the ordinary move. Refusing the collision, as
[`Acting on a whole filter`](#acting-on-a-whole-filter) has *Make a package* do, would mean
forgetting and retyping to do the common thing. What a replacement must not be is silent. It is the
one act here that takes something away, so it counts as nothing, shows both queries and asks. Two
rows under one name is the state the rule exists to prevent — a strip cannot tell them apart.

**And the confirmed write uses what the confirmation showed, not the bar as it then stands.** The bar
is live while a confirmation sits on the page, so the filter can move between somebody reading the
sentence and pressing the button. Same rule as every filter-wide action one section up, at a much
smaller stake and for the same reason.

**A chip rewrites its own filter, and that one does not ask.** A curation pass is a name given early
and narrowed all morning, and *Portuguese, unclassified* means something different by eleven than it
did at nine. So writing the narrowing back is the commonest thing anybody does to a saved filter.
Doing it through the save box means typing a name on the screen, then answering a question about a
collision that was the whole intention. The chip carries the button instead.

**What separates it from the paragraph above is how the row was arrived at, not what is at stake.**
A save reaches an existing row by colliding with it, and the confirmation's work is to say *which*
row that is. A name typed in a hurry can land on somebody else's morning. A button drawn on a chip
has already named its row; there is no second filter it could have meant. What is still owed is the
sentence, and it names the filter rather than either query. A line of `language=pt&favorited=out` is
the machine's spelling of a question asked in somebody's own words; reading it tells them nothing
they did not do.

**Every sentence the saved strip says is a toast, and the one thing that is not a sentence stays in
the slot.** These controls sit above a page of rows. Their message slot has nobody watching it and
nothing to clear it, so an hour-old rename still reads as something that just happened. A toast is
seen where it is raised and then goes — owed to an act somebody took deliberately and can see the
result of. That reaches the save box as much as the chips: the same strip, the same slot. A save is
the act most often repeated.

What the slot keeps is the collision confirmation, which carries two buttons and the two queries.
A toast fades, and a control that fades is one somebody reaches for and misses.

**A saved chip is the accent, not the green the showing chips are.** The two bands are a name
somebody gave a question and the filter narrowing the list right now. These are different claims, so
one colour over both leaves the bar saying them in one voice. The accent, rather than a colour of
its own, because the chip is a link pressed to go somewhere. That is what the accent means on every
other page.

**The rewrite keeps the filter the kind of filter it was.** The save box has a *keep the page* tick,
and a chip has nowhere to put one. So the answer is read off the rewritten row: a query carrying an
`offset=` is a place somebody works from and is rewritten with one. A query without is a question,
and stays a question. Offering the box on a chip would ask, on every rewrite, a question that was
settled when the name was given.

**Renaming refuses a name that is taken, where saving replaces one.** The two are the same act
pointed opposite ways. A save writes a *query somebody is looking at* into a name, so both queries
can be put on the screen and a choice offered. A rename writes a name over a query that is not on
screen and belongs to a row nobody is thinking about. There is nothing to show in a confirmation, so
the row would simply go. So it is refused in words, which is what *Make a package* does with the
same collision.

**The box opens inside the chip, and the answer is the whole strip.** Inside, because the strip is a
wrapping row and a name being typed anywhere else would move every chip after it. The whole strip,
because the order is the fold of the name. So a rename can carry a chip past its neighbors, and a
redrawn chip alone would sit in the position its old name earned.

**The page comes back by default, and a box lets somebody leave it out.** *The page comes back too*
in the section above is the whole argument: page sixteen of one favorite is a place in a morning's
work. The box is for the other kind, a filter naming a *question* rather than a place. That kind
should open at the top, however deep the corpus was being read when it was saved.

**Restoring is a link, not a swap.** Every control in the bar has to come back set, and the bar is
not inside `#rows`. So it is a page load, exactly as a chip's `×` is. The link carries its `?`
whether or not anything follows it. A bare `/songs` is answered with the remembered filter, which is
the opposite of what a saved whole-corpus filter means.

**A favorite that has gone is dropped as the strip is drawn, and the row is left alone.** Every other
part of a saved query is text: a fold, a language code, a tag slug, matching nothing at worst, in
its own words. A favorite is a row id, so one deleted since the name was given is an empty page with
no explanation. Once ids start being reused, a chip names the wrong favorite confidently. Every
control on the bar reads a value it does not recognize as *any* for that same reason: see
[`What the browse bar's numeric filters offer`](#what-the-browse-bars-numeric-filters-offer). The
stored row stays as typed: editing it because a favorite is missing today is wrong once it returns
from a backup.

**Saving is the one action on that page that does not send the filter bar with it.** Every other one
reads the bar out of its own body, for the reason
[`Acting on a whole filter`](#acting-on-a-whole-filter) gives. This one does not need to: every
change to the bar goes through the rows route, which writes the query string down before it answers.
So the server already holds the string the address bar is showing. That costs a dependency: a route
that redraws the rows without writing the filter down strands this save on a page nobody is on.
Three routes redraw them, and a test holds each.

## What the browse bar's numeric filters offer

**Bands that partition the column, never a ladder and never a union.**

A `≥ N` ladder returns nested sets, so adjacent options show largely the same songs, and it has no
upper edge. So **there is no way to ask for the bad files at all**. The one question a curator most
wants answered on a fresh corpus is *what is broken here?* That is the one shape the control cannot
make. **Suitability** is three bands, `8-10`, `5-7` and `<5`, which between them cover 0–10 and
overlap nowhere. **Copies** is `1`, `2-10` and `more than 10`, which partition the column; `2 or
more` is not a fourth bucket but the union of the two after it.

What that costs the dropdown is precision. No option says exactly `≥ 9`, and none says *two or more*
in one click. In both cases the capability sits somewhere else. **Sorting by suitability answers a
threshold better than a threshold does**, because it shows where the cliff falls instead of asking
somebody to guess. The *Duplicates* page is where "more than one copy" is the real question, and it
links to exactly that.

**The `suitability` parameter takes any range, and the dropdown still offers the bands.**
`suitability=2-5`, `suitability=9`, `suitability=7-` and `suitability=-4` are the four spellings, and
each narrows to what it says. A range is a question somebody asks a few times in a corpus, and never
from a control. A fifth standing option would overlap the three bands, and would take from all of
them the shape a reader can hold. An address is where a question that precise belongs.

**A range in force is the last option in the select, and it is there only while it holds.** The four
standing options are the same four whatever the address says. What a range owes the page is a control
that agrees with the rows. A select reading *any* over a narrowed list is the fault the chip strip
prevents one element up the bar. Picking a band submits the bar, and the extra option goes with it.

**Both ends lie in 0–10, the low end is no higher than the high one, and anything else reads as
*any*.** An open end takes the end of the column, so `7-` is 7 to 10. A range whose ends are a band's
ends is that band. So `-4` and `0-4` ask one question, draw one chip, and are written back one way.

**`0-10` is not *any*.** A song with no stored suitability falls outside it, because `NULL` answers no
comparison, and that is the rule every band already follows. *Any* adds no clause and holds the whole
corpus. `set` and `unset` on the personal score are what ask about a number that is missing.

**`CopiesFilter::AtLeastTwo` is the one bucket the bar cannot show, and it is still spelled.** The
dropdown does not offer it, being the union of the two below it rather than a fourth of them. The
link from the Duplicates page needs it: a value defaulting to *any* would quietly answer
*more than one copy* with the whole corpus. So it parses, it round-trips, and it draws a chip like
every other filter. A filter narrowing the list with nothing on screen is the fault this row exists
to prevent: a control disagreeing with the page it controls.

The parameter is `suitability` rather than `min_score`, because a name that says *minimum* above a
control offering `<5` is a name that lies. An unknown key is ignored and reads as *any*, and no
retired key is ever written back out. So a link normalizes on its first page turn: one filter, one
spelling. See `No compatibility aliases` in [`songs.md`](songs.md).

## Languages can be left out, several at once

**The bar narrows *to* one language and *away from* any number of them, and they are two controls
because they are two questions.** A corpus holds folders in languages the person curating it does not
read. Those folders are in the way of every browse list, every count and every filter-wide action.
*Portuguese* and *not Vietnamese, not Thai* compose: one select cannot hold both. A single exclusion
is the wrong size for the problem, because a corpus that has one such folder has several.

**A language nobody has placed is not in any language, so it stays.** `NOT IN` over an empty column
answers neither true nor false. That would take every unclassified song off the page with the
language asked about, most of a corpus mid-classification, with nothing on screen saying why. The
clause says so outright.

**The exclusion writes nothing.** It narrows what is being looked at and is gone when it is dropped,
which is what makes it the cheap thing to reach for. A judgment meant to outlive the session is a
tag or a favorite, and those already travel into a package.

**One chip per language, not one chip for the set.** A chip over a set can only offer *drop all of
them*, and taking one language back is the ordinary act. That is the same reasoning, and the same
`<key>:<value>` spelling, that `Assigning tags in bulk` settles one control over.

**The picker offers the languages the corpus holds**, not the 184 the standard has. Offering to
leave out a language no song here is in is an option that changes nothing.

## A favorite does not nest

**A favorite is one named list, and a collection divides by naming more of them.** `Brasil Axé`,
`Brasil Samba`, `TODO Rock` — the grouping lives in the name, where it is readable in every control
at once and costs nothing to change.

**What a container would have to be, to be worth having, is a thing that filters.** Filing a song
under `Brasil / MPB` and then asking for `Brasil` has one honest answer: the songs under it and
everything beneath it. It must agree across four places: a recursive query behind every browse page,
the count beside every list, and both halves of the bulk file. That has to hold forever, about what
*in this favorite* means, so that one control can do what a name already does. A container that does
not filter is worse than none: it reads as empty, it narrows to nothing. The page gives no sign
which of the two it is.

**So the filter matches one list and is complete.** *In this favorite* has one reading, the same one
the star files under and the same one a backup carries.

**A name is unique across the collection**, which is what lets every control label a list by it and
what a backup matches on. Two lists somebody would have told apart by where they sat are two names.
Having to pick the second is the moment to notice they are the same list.

## A favorite can be a working list

**Some lists are scaffolding for a later pass rather than a filing**, and a favorite carries one flag
saying which. *To check*, *language unclear*, *maybe for the party* are containers built while
going through a folder, so the deciding can happen in one sitting afterwards. The songs in one have
been set aside rather than put anywhere.

**The browse row's star stops claiming they were filed.** The fill
still says the song is in a list, because it is. The color stops, because gold down that column
means *done*, and a song waiting to be decided about is the opposite of done.

**The chooser under a row offers the filings first, and the working lists under a divider naming
them.** Filing a song is what the chooser is for; a working list is where a song waits, so the
two are not interleaved by name. The Songs page's favorite filter orders them the same way, working
lists in their named group, and the Favorites page's table draws the same divider. Each group lists
its rows in name order. Everything else about a working list is unchanged: it holds songs, filters,
backs up, and its songs go into packages like any other favorite's. A second kind of list would have
been a second mechanism; this is one word about an existing one.

**The flag is on the list, not on the membership.** What makes a song's presence provisional is the
list it is in, and it says the same thing about every song in that list. A flag per membership would
let one list be half scaffolding, which is a distinction nobody drawing up *to check* is making. And
it would put the answer somewhere a person has to visit a song to change.

**Set on the Favorites page and nowhere else.** What kind of list this is is a judgment about the
list rather than about the song being filed. So it belongs where the lists are read side by side:
*to-check* beside *Bossa nova* is the comparison that answers it. Offering the box wherever a
favorite can be made would ask the question at the moment somebody is thinking about a song. It
would also put three copies of one control on three pages.

**No confirmation, and no Save.** It writes one flag, takes no song with it, and ticking the box back
makes the list exactly what it was. So the second click is the whole of the undo. A dialog in front
of an act that undoes itself is a dialog nobody reads.

**A favorite is a filing until somebody says otherwise.** The flag defaults off, so a list nobody has
ticked is a filing. Nothing takes a column of gold stars off a curated corpus by guessing.

**A restore says what kind of list one is only where the file wins.** A backup carries the flag like
every other thing a person typed. A restore that creates a list creates it as what the file says.
One that finds the list already here leaves it alone unless the policy is *the file wins*. The flag
carries no value meaning *nobody has supplied one*. False is a decision as much as true is, so
filling one in is overwriting rather than completing.

## A favorite's page shows every song filed in it

**Narrowed to one favorite, the song list shows every song the list holds, second versions
included.** Its row count is the count the Favorites page links from.

**Collapsing versions decides what the corpus offers for filing, not what a list holds.** It hides
the other versions of a recording so that nobody stars six copies of one song. Inside one list, the
same predicate hides songs somebody already filed. A song filed as a second version, or a song that
became one when the grouping pass ran later, then vanishes from its own list. A list made only of
such songs reads as empty beside a count of ten, and nothing on the page says why.

**Two versions of one recording in a list are answered on the Favorites page**, by the *second
copies* count and Tidy beside it. The song list does not answer the same thing a second way, by
hiding one of them.

## A hidden version says so on its row

**A row the song list would hide carries the group's version count and a *hidden* mark.** The
mark links to the version shown in its place. Such a row appears through *every version*, a
favorite, the similar-names page and the lyric search.

**Without the mark it reads as a song in no group.** The count lives on the shown version alone, so a
hidden row said "—", the same as a file nothing resembles. Somebody filing it had no sign that the
list offers a better copy, or which one.

**The count is read off the shown version rather than written onto every row.** The grouping pass
already writes it there, and a stored copy on each hidden row would need a schema step. A database
opened by that step could no longer be opened by an installed build of the tool.

## Filtering by whether a song is filed

**Five options: *any*, *in any favorite*, *in a filing*, *in no favorite*, *in no filing*.** **The
ones that are not *any* or *all* are what earn the control.** A checkbox can only ask *which of
these have I filed?*
The question a curation pass is actually made of is the other one: *what have I not looked at yet?*
On a corpus where almost nothing is filed, it is the arm that turns every row there is into the work
remaining. The two are not each other's absence either: no filter at all shows both.

***In a filing* is that same question asked part-way through a pass.** A working list holds songs
somebody set aside to decide about later. So *in any favorite* counts the undecided with the
decided, and cannot answer *what have I settled?* The browse row already draws the line one song at
a time, filling a star for any list and colouring it only for a filing. This is that line as a
filter. See [`A favorite can be a working list`](#a-favorite-can-be-a-working-list).

***In no filing* is its other half, *what is left to settle?*.** *In no favorite* cannot answer it: a
song set aside in a working list is in a favorite. So it drops out of the work that remains. *In no
filing* shows that song beside the songs nobody has looked at.

**The flag is read off the list, so a song in a working list and a filing is filed.** And *in no
filing* leaves it out. One list saying *not yet* does not unsay another list's decision about the
same song. That is what follows from the flag being on the list rather than on the membership.

**Beside the picker that names one favorite, not folded into it.** That control answers *which
favorite* and this one answers *any at all*: two questions, two controls. They sit in the same band,
so the difference is visible. One select holding both would put a list of names under two options
that are not names. *In no favorite* has nowhere to sit among them.

The parameter is `favorited`, and its values are `in`, `filed`, `out`, `unfiled` and `1` — the
spelling a checkbox sent. That value is read and never written, so such a link narrows the list it
was written for and normalizes on its first page turn. Same rule as `suitability` above, and the
same one in [`songs.md`](songs.md#no-compatibility-aliases).

## Filtering by one artist

**A box in the bar that means *exactly this performer*, and every artist in the list is a link that
fills it.**

**Exact, where the box beside it is a substring search, and that is why it is a second box, not a
change to the first.** *Title or artist* answers *which song is this?* `Queen` there brings back
Queensrÿche, every tribute act and any song with the word in its title. That answers one song and
misses the other question a curator asks constantly while filling a package: *what else did they
do?* Two questions, two controls, sitting beside each other in the same
band so the difference is visible rather than something to discover. The same argument
[`Searching lyrics`](#searching-lyrics) makes one control over.

**Matched on the folded key, so it is exact about a performer rather than about a spelling.** The
filter compares `sort_artist`, which is `km_song::text::fold` of the effective artist. That is the
alphabet the A–Z strip files by, the song book prints and both catalogs sort by. On a real corpus,
one performer arrives under four spellings across four folders, so an exact raw-column match only
answers *what else did they do?* with a quarter of the answer **while looking as though it had
answered**, which is the worst shape a filter can have. Folding also makes the box forgiving of case
and accents, which an exact filter otherwise is not.

**A song with no artist matches nothing, and there is no *unset* option.** `sort_artist` is NULL for
one, so it falls out of every artist filter rather than joining a filter for the empty string. That
is not an omission. A song nobody recorded an artist for is not *by* anybody, so the question this
control asks does not apply to it. The em dash in its row is not a link either. A link to `?artist=`
would be a control that looks like it leads somewhere and matches nothing.

**The link replaces the filter rather than adding to it, and that is forced rather than chosen.** One
row's markup comes from three places, and none of them sees the browse query:

- the browse list;
- the single-row fragment that a rename, a score or a star swaps back in; and
- the lyric-search hits.

(The same fact makes the file-name switch a class on `#rows` rather than a field on a row; see
[`What a browse list shows without being asked`](#what-a-browse-list-shows-without-being-asked).) So
a row cannot know what else is narrowing the list. It is also what every other filter link in this
tool already does: the Folders page, the Favorites page, and a package's *not packaged* link. The
chips strip says what happened either way, which is the whole reason
[`Acting on a whole filter`](#acting-on-a-whole-filter) insists every filter has a chip.

**The row's artist box is `row_artist`, and that is the fourth time this trap has been paid for.**
`#rows` rides in the same body as `#filters` for the ticked-song actions. Serde answers a repeated
*known* key with `duplicate_field`, so the bar owning `artist` means a row may not. It is
the rule that already made the bulk language set `set_language` and the row's own select
`row_language`. How it would have failed is worth naming. A row's editor is in the page while a row
is open, so the collision turns five buttons into an intermittent 400 that looks unrelated.

**The bar owns the plain spelling and a control inside `#rows` takes the prefix.** Only what collides
is renamed — the `title` box beside it keeps its name, because there is no title filter for it to
collide with.

**No index of its own.** This is a residual predicate over whichever sort index the query is already
walking, like `kind`, `granularity` and `encoding_source`. `Db::create_browse_indexes` serves the
nine *sorts*, and nothing here changes what that list has to hold.

## Throwing a song away

**A curator can say a song is not worth keeping. That means it leaves every list, and its files are
not read again.** A corpus holds a bad rip, a copy the clustering could not match, and a `.kar` whose
lyric track is the arranger's credits. The only other answer is to take the file off the disk. That
is a different decision, about somebody's files rather than about their catalog, and not always
theirs to make.

**It is a third way of hiding a song and must not be confused with the two above it.**
[`merged_into`](#duplicate-aggregation) is a person saying two files are one recording.
`duplicate_of` is the tool's guess at the same thing; both answer *which copy do I show*. This one
answers *does this belong in the catalog at all*, which is why it survives a clustering pass that
rewrites every group from nothing. A song set aside by a guess comes back when the guess changes,
and one somebody discarded does not.

**The bar has one box for it and there is no *both*.** Every other control there narrows a list of
songs being curated; this one chooses which of two lists is on the page. A deleted song carries a
rating, a filing and a package that nobody means any more. Mixing the two would put rows into
every count and every page that none of the other controls can say anything useful about. It is a
filter and not a view: it changes which songs match, so turning it on starts again at the top.
*Clear all* takes it off, where the two view boxes survive one.

**The scan does not read a deleted song's files, and `--force` does not reach them.** Every other
skip in a scan is an optimisation: the file would be read to arrive at the row already stored. A
forced run exists to overrule them when a heuristic moves. A song somebody discarded is not
waiting on a better answer, so this is the one skip a forced run keeps. On a corpus this size, the
reading it saves is most of what deleting is for.

**The file rows stay where they are.** *Which files are gone* is what the walk found subtracted from
what the database holds, and a skipped file is a file the walk found. So nothing forgets its
row, and bringing a song back needs no rescan to find it again. Undelete is beside Delete in the tab
for the reason [`Assigning tags in bulk`](#assigning-tags-in-bulk) gives about direction. One button
that discarded a song or restored it, depending on what the row already was, would be a single tick
meaning two opposite things.

**A song a package names goes like any other, and the confirmation counts how many.** Deleting one
takes it out of the list that package is rebuilt from. The number is on the screen before the button
is pressed, and the judgment belongs to whoever curated the package. A refusal here would mean a
corpus cannot be tidied until its packages are emptied first, which is the wrong way round. This is
the one place the builder parts company with the scanner's rule: a file stays gone from disk while a
package names the song. That is a drive that went away, and this is somebody saying so on purpose.

**A discarded song says so on its row, and it is the first chip there.** Browsing hides these. A row
carrying the chip was reached through *only deleted*, a saved filter naming it, or a tab left open
across a delete. The row is otherwise identical to a live one, and every action beside it is live.
The star files a song no package will take, and the play button opens a file no list offers.

The chip is filled rather than outlined, which this tool spends on a mark saying what a row *is*. An
outlined one says what the analysis found. Every browse row selects `deleted_at`, for the reason the
warnings are selected: a row redrawn on its own never sees the query that found it.

**The star stays on and every read leaves it out.** A delete writes one column and clears no filing,
because a filing is what a restore has to give back. A package therefore reads its members and its
sourcing lists through the discard. A sync counts a thrown-away song out rather than keeping it, and
a package built afterwards ships what the page shows. Taking the stars off instead would make
undelete return the song and not its place in anybody's list.

**Replacing a song in a package with one that was thrown away is refused.** A merged song already
earns a refusal, and this one stands beside it for a harder reason. A merge leaves a survivor
standing in the song's place, so the refusal names it and somebody picks it. A discard leaves
nobody. The replacement also stars the substitute into the lists a sourced package follows.
Accepting one would put a song into a build through the side door the sync closes at the front.

**It keeps the page, alone among the bulk actions, and answers with the table.** The others write
*onto* songs that keep matching the filter that found them, so the rows stay true and a toast is
the whole answer. This takes songs out of the list they were ticked in, so a table left as it was
would offer rows that have gone. The page rides in the frozen query string the confirmation
writes, the same mechanism that freezes the filter. A page the write pushed past the end comes
back as the last real one. That is
[`What keeps the page you are on`](#what-keeps-the-page-you-are-on) applied to a write that changes
how many songs match, though the count stays behind, for that entry's other half.

**Deleting is hand curation, so a backup carries it and the edit stamp moves with it.** These are
the two things `merged_into` already gets, and for the same reason. Both are a person deciding a
song does not belong in a list, and a scan can work out neither. A whole corpus of judgment about
what to keep is exactly what a backup exists to hold.

**A restore never brings a song back.** In either direction, a file saying nothing about a song
leaves a deletion made here in place. Returning a song to the catalog is a decision somebody makes
on the page.

## Test-playing to a machine that is not this one

**A machine on this box is handed the song's path; a machine anywhere else is handed the song.**
`POST /debug/play-file` carries a path, and the machine opens it on *its own* disk. Curating a
corpus on a desktop and hearing it on the appliance under the television is the arrangement this
project is built for. That otherwise fails with "is not inside an allowed folder." The refusal's
advice cannot work: the folder it names to add to `play_file_roots` does not exist on that machine.

**The two routes are not a fallback pair, they are two correct answers.** That is why the choice is
made from the address rather than offered as a setting. A path is instant and copies nothing, and a
loose video song is hundreds of megabytes; over the network a path means nothing at all. An address
that will not parse counts as remote. An upload to a machine on this box copies a file
needlessly, where a path to one that is not does not work at all.

**The pair travels together, under one name.** An MP3+G song is a `.mp3` and a `.cdg` sharing a stem,
so both halves are sent in one request. They stage under a single `stem` field rather than under
their own filenames. That is not tidiness: a multipart filename is a header, and its encoding across
the wire is not something either end can rely on. `km_kmpkg::pair_for` documents a measured pair
whose stem ends in a **space**, which Win32 strips from a path component. So two halves named from
two filenames can end up unable to find each other.

The machine's side of this, including why it ships refusing uploads, is `A machine takes no uploaded
song until it is told to` in [`api-and-network.md`](api-and-network.md).

## Installing a package makes the same choice test-play does

**A machine on this box is handed the `.kmpkg`'s path; a machine anywhere else is handed the
`.kmpkg`.** Sending a path to a machine that does not share this filesystem answers **`No such file or
directory (os error 2)`** — a sentence about *its* disk. On a screen, the only file anybody had
been thinking about was the one this tool had just written.

**Uploading a whole volume is affordable.** A package is gigabytes and wants resumability, progress,
and a check that the appliance has room. But the machine's upload route caps a package at just
under 2 GB and stages it to disk in chunks rather than buffering it. The fifteen-minute timeout
the song upload uses covers a package over a house's Wi-Fi. Resumability and a free-space check are
real work for a transfer somebody does a handful of times per volume, at a desk, watching. A plain
upload that says what happened is worth more than a perfect one that does not exist.

**Two guards come with it.** Before anything is sent, the file is checked to be there. `tidy` falls
through to the path unchanged when it is not. A package whose `.kmpkg` has been moved or deleted
produces exactly the same os error 2 from a machine that is working perfectly. It names the path and
says to build it again.

**The second is the machine's password.** Sending a package and installing one the machine can
already see are both admin actions, so both need it. This tool has somewhere to keep one: a
password box on the machine panel, and a token held for the run. That is the same policy and the
same shape `km-admin` already has.

**The token is on the server's shared state rather than on the client**, which is not a detail of
where a field went. This tool builds a fresh client for every request. It has to, because which
address to use is a read of the database and of what the network is saying. So a token owned by the
client is dropped between the form that obtained it and the button that needs it. The box would have
appeared to work exactly once, on the page that took the password.

**The refusal is still there for somebody who does not know the password.** It names three ways:
type it here, open the owner's page on that machine, or copy the file into its packages
folder and press Rescan. **Both send routes say it.** With only the upload half explaining itself,
one missing password reads as three paragraphs of instructions on a machine across the house. On a
machine on this desk it reads as `'…' needs the admin password; send an admin token`.

**And the switch beside it is the one `explain_uploads` names.** A machine ships with debugging off,
so the first test-play against one that is not this computer is refused. That refusal says the
quickest way through is a Debugging button on this panel. It is on the panel, in the signed-in half,
because turning it on is an admin action too.

### The password is remembered per machine, or not at all

**Off by default, in a file of its own, keyed by the machine's id.** All three halves are `Where a
key somebody typed into a page lives` in [`repository.md`](repository.md), which `km-admin` already
follows for provider keys. This is that policy one tool over rather than a second one.
`machine-passwords.json` sits in memory unless the box is ticked, beside a tool's other per-user
state, and holds credentials and nothing else. Forgetting deletes the file rather than emptying it —
a `{}` left behind reads as *a password is remembered here*.

**Both tools that meet the closed door keep one, on identical terms.** Each puts it where that
program already keeps a credential somebody typed in. `km-package-builder` uses its per-user config
directory, named by an environment variable because it has no other way to point a run elsewhere.
`km-admin` uses **its own data folder**, beside `provider-keys.json`, because it is given that folder
on the command line already. Naming the same thing twice would be a second way to say it. Neither
reads the other's.

**The store is one crate rather than two copies.** The two programs sit on opposite sides of a cargo
workspace boundary. A shared crate taking no HTTP client is the only way they can share anything
at all.

**A folder given on the command line is what makes a test and a smoke run safe.** It is stronger
than a flag in the source: a `cfg(test)` guard is per crate. That guard is off in exactly the
integration tests that drive a whole program. A path that arrives as an argument cannot be got wrong
that way.

**A machine handed over with `--machine` is remembered no more than its address is.** That address is
deliberately not written down, so there is no record for an identity to attach to and the box is
absent. A one-off address carries no persistent state of any kind.

**Which means the record has to be read against the machine in force rather than trusted.** A record
is on disk whatever this run was pointed at, naming whichever machine was chosen last. An id
taken from it regardless would key one machine's password and then spend it against another. So both
the reading and the offering ask first whether the record is about the address being used.

**Not in the `.kmbuild`.** The machine's *address* is in there deliberately: a corpus is a
document, and a second computer opening it should reach the same machine. That is
what disqualifies it for a password: a document travels with the corpus, onto an external
drive and to whoever is handed the songs.

**Keyed by the id rather than the address**, so a machine that moves is still recognized — the same
anchor the follow uses. The consequence is that a machine which has never answered cannot be
remembered, and the checkbox is not offered rather than offered and ignored. There is no
identity yet to key a password under. A row shared by every anonymous machine would be a password
handed to whichever answered next.

**A remembered password is a standing instruction to sign in, not a sign-in that has happened.** It
is spent at the moment a token is wanted rather than at startup. Starting up would put this tool on
the network before anybody had asked it for anything. A token expires, where the instruction does
not. Failure is silent, because what follows it is the machine's own refusal and the sentence saying
how to sign in. That is a better answer than a second error about a password somebody may not
remember setting.

**An unticked box forgets the password typed beside it.** The checkbox is a statement about what this
computer should be remembering rather than an action taken once. Signing in with it clear removes
whatever was remembered before.

**It says nothing about a pass that typed no password.** Both surfaces draw the box only where there
is something to type. A password already saved is a way in, and offering it again asks for what the
program is holding. The press that spends what is saved leaves no tick, so absence would read as
*forget* and delete a credential as a side effect. The route out that
does not run through signing in is *Forget it*, beside the sentence saying there is something to
forget. That is the state somebody trying to stop is already looking at.

**The token is checked before a byte is sent.** A 401 arriving mid-stream reads as a dropped
connection to the sending half. The refusal a curator most needs would then be the one least likely
to arrive. That is the same failure the pre-flight on `/discover` already exists for, and a 300 MB
package never wins that race.

## Both roads to an install say the same sentence, and it is written once

**The wording lives in `InstallReportDto::sentence`.** Both client calls returning
`serde_json::Value` puts the raw body into `{report}`. An install then reports a sentence with the
JSON it came out of stapled to the end. The loopback branch is worse, because its JSON is less
obviously JSON and reads like a diagnostic.

**The two routes answer different shapes on purpose.** A machine sharing this filesystem is handed a
path, and `POST /admin/packages` answers `InstallReportDto` — fields, for a program. A machine
anywhere else is handed the bytes, and `POST /admin/packages/upload` answers `UploadReportDto` in
prose, because only the machine knows what it did with them.

**One sentence, one spelling.** `installed "Carols 1999" · 155 songs` is otherwise written out
character for character in three places:

- `karaokemachine::dropped` (a package dropped on the window);
- `Machine::accept_upload` (a package uploaded to it); and
- `karaokemachine::handed` (a package double-clicked while the machine is running).

One crate, one sentence, three spellings: that is how three roads to an install come to report the
same install three different ways. Each reads the DTO it can build or decode for free.

**The quotes hold the package's name.** An id is sixteen hexadecimal characters from the operating
system's entropy, and the readable half is what every surface shows — see
[`A package's id is generated, not typed`](packaging.md#a-packages-id-is-generated-not-typed). The
report carries the name out of the manifest the install just read, so no route has to look one up.
Behind it is the id, for the manifest that names nothing and for a machine older than the field. A
package with no label is still reported as something.

**Only the path branch can name duplicates, and it does.** `duplicate_content` — the same recording
already in the catalog under another number — is the one thing the fields carry that the prose cannot.
The person who has just built the package is the only one placed to act on it. It is appended
through `first_ten` for that helper's own reason: a message slot is read at a glance. A package
duplicating a hundred songs would otherwise bury what it did under a list of what it did not do.

## The install button comes back with the build that made it usable

**The last frame of a build that wrote something brings a fresh install form with it.** The button's
`disabled` is rendered from `built_at`, read when the page is drawn, and a build swaps
`#build-progress` and nothing else. So a build finishes, prints a message directly above the button
saying what it wrote, and leaves that button grayed out with `Build it first.` beside it. Two
statements contradicting each other on the same screen, and the wrong one attached to the control.

**Out of band rather than by widening the swap.** This is the arrangement `filter_chips.html` already
uses for a strip that lives outside the rows describing it. The form is a sibling of the progress
fragment, so it carries its own `id` and `hx-swap-oob`. It is a fragment of its own for the same
reason the remote pre-renders a row's star. The copy a swap brings in has to be character-for-character
the copy the page was drawn with. Two spellings in two files is how they come to differ by an
attribute nobody notices.

**The build's own report decides, not a second read of the database.** It is already what decides the
message above the button, so asking anything else could only produce a page whose two halves disagree.

## A failure is accepted per file, so a new one is still news

**A Remove button on each row of `Files that did not parse` accepts the files failing that way, and
a Restore below the table puts them back.** A real corpus fails in ways nobody is going to fix: an
MP3 with no `.cdg`, a video in a build with no video feature. A list that goes on
reporting them is one nobody reads. A list nobody reads hides the failure that mattered.

**The files, not the reason.** Eight orphan `.cdg` files somebody has been through are settled; a
ninth appearing is the news the list exists to carry. So removing accepts the files failing that way
at that moment, and a file that starts failing the same way later is counted separately. Rejected:
dismissing the reason is one line less code and makes the ninth file silent, which fails in the
direction nothing can notice. A message that never appears looks exactly like a corpus with nothing
wrong in it.

**A verdict names the failure it was passed on.** A file accepted as unreadable that later fails a
different way is counted again, because the two are different things to know. A dismissal is a
verdict on one failure rather than a permanent exemption for a file.

**The count in the bar is failures nobody has accepted.** The red badge says there is something to
go and look at, so it has to agree with the list. Counting an accepted failure would leave it lit on
every page above a scan page with nothing left to show. The only way to put it out would be to
make the files parse.

**A rescan does not undo it.** The tally is recomputed from `files.scan_status` every scan, and the
dismissal is stored beside it. That is the arrangement a dismissed duplicate pair already uses, and
for the same reason. A judgment a person made must outlive the pass that proposed the thing they
judged.

**Removed reasons are listed under the table rather than hidden.** They carry their counts and their
Restore buttons behind one line saying how many there are. A removal nothing records is a one-way
door, and the folder still holds the files. The line below the table says so in as many words. So
`Files` on the Settings page and the scan list cannot be read as contradicting each other.

## A rebuild raises the version, and it is the patch that moves

**A tick box on the build form, on by default, raises a package's patch number on every build that
writes a file.** A package's version is a label for a person. An install is keyed on the package id,
so reinstalling *upgrades* rather than clashing — see
[`A package's id is generated, not typed`](packaging.md#a-packages-id-is-generated-not-typed).
Nothing in the machine compares two version strings.

The version's only job is to tell two
`.kmpkg` files apart. One that never moves cannot do it. A volume rebuilt with fifty more songs
in it claims to be the same 1.0.0 as the one before it.

**The patch, because whichever number a build moves stops saying anything a person chose.** `Z`
counts builds since an edition and `Y` stays the curator's, to raise when a rebuild really is a new
edition. Rejected: moving the minor reaches `1.40.0` in a fortnight of ordinary rebuilds and leaves
nothing below the major to declare an edition with.

**The first build is exempt.** A package ships at the version somebody typed. Every build after
that raises the version before it writes, so the field on the page always names the file on disk.
Rejected:
raising afterwards, ready for next time, means the number in the box describes a package that does
not exist yet.

**Only a build that wrote something spends a number.** The raise happens in the same lock that
records the build. A refusal, a failure and a stop all leave the version where it was. A corpus
would otherwise drift its versions forward every time somebody hit the language gate.

**Kept per package, in the curation database**, beside `default_language`. The reason is the same:
it is a fact about a package, not a habit of a curator. So a second machine pointed at the same
drive agrees. The column is `NOT NULL DEFAULT 1`, so a package nobody has unticked is opted in
rather than silently opted out.

**On by default, against the rule that a checkbox here is off by default.** That rule —
[`What a browse list shows without being asked`](#what-a-browse-list-shows-without-being-asked) —
buys the collapse of "unticked" and "a page that never had the box" into one meaning, and it is
still what reads the box. What it does not settle is which way the default points. Here the two
sides are not equal. An unraised version is silent, discovered later by somebody holding two
files that claim to be the same package. A raised one nobody wanted is a number in a field
directly above the box, editable in a second.

**A version this tool stores is three numbers with dots between them.** The New package and Version
boxes refuse anything else. A build can only raise a version it can read, and a box that
quietly did nothing on `2024-spring` would be worse than no box. **A package opened from a `.kmpkg`
built elsewhere keeps the string it carries.** Refusing an import over a label would throw away the
songs to save the string. So the build page says that version cannot be raised, and one edit makes
it one that can.

**The label carries the number, not the rule** — `raise the version to 1.0.4 on this build`. A box
saying only `raise the version` leaves the reader guessing:

- which digit moves
- whether this build or the next one moves it
- what becomes of a version that is not three numbers

## What the tool calls the package it writes

**`<name>-<version>.kmpkg`, in the corpus's data folder, and the box on the form may be typed over.**

**The version is in the name because that is the only place it can do its job.** A version's whole
purpose is to tell two files of one package apart — nothing in the machine compares two of them. A
name that leaves it out means the second build overwrites the first; the manifest's number is the
only record that they differed. See
[`A rebuild raises the version`](#a-rebuild-raises-the-version-and-it-is-the-patch-that-moves), which
the name is what completes.

**The folder is one field and each box holds only a file name.** The package and its description go
to the same folder by default. A full path in a box puts the name past its right edge, which is
the part somebody is reading for. A blank folder is the data folder.

**`<name>` is the volume's name.** It is the package's name while the package has one volume. Once
it has two, the volume number follows the name — see
[`A package holds volumes`](#a-package-holds-volumes). Two volumes of one package therefore never
write one file.

**The name and not the id.** The id is sixteen hexadecimal characters, and a folder of those is a
folder nobody can read. Where the id is needed is on the machine, and the machine derives its own name
without being told — see
[`What an installed package file is called`](packaging.md#what-an-installed-package-file-is-called) —
so nothing downstream depends on this. A name that folds away to nothing falls back to the id, which
is always a legal file name.

**The description beside it is `<name>.kmspec.yaml`, by the same rule and without the version.** Two
files land in that folder saying the same thing about the same package. A reader who can pick the
`.kmpkg` out by eye can pick out what describes it, too. An id there would break that: the
description would follow one naming rule while the `.kmpkg` above follows another. The version is
absent because a description is of the *package*, where
a `.kmpkg` is of one build of it. A rebuild writes another package file beside the first and
rewrites the one description.

**It is the version the build is about to write, not the one stored.** The raise happens before the
write, so the two are different numbers on every rebuild. The tick box's label already names the
raised one, and the file name comes from the same answer. A page that worked it out twice would put
a name in the box that no file on disk ever had. Moving the box moves the name with it.

**Every build now leaves a file where each used to overwrite the last, and that is the cost.** On a
corpus of video packages a build is tens of gigabytes, and a fortnight of rebuilds is a folder nobody
meant to fill. It is accepted rather than answered, for two reasons:

- holding two builds and being able to say which is which is the point
- `_kmbuild-data` is the curator's own folder on a curation workstation rather than an appliance

A tool that deleted last week's build to save room would be deciding something the curator is
better placed to decide.

## Discovering a machine in the package builder

**A Discover button on the Settings page lists what is advertising itself, and every entry is a button
somebody has to press.** The tool talks to a running machine for two things: test-playing a candidate
and installing a package. The loopback default is right only when the machine is on the same box.

**Listing is not setting, and that is the decision rather than an implementation detail.** This tool
installs packages. A version of it that re-pointed itself at whatever answered a browse first would
eventually install somebody's package on the wrong machine. A house with a machine under the
television and another on a desk is not hypothetical here. Discovery is asked for by pressing a
button and applied by pressing a second one. The fragment holds nothing that can fire on its own —
a test asserts that.

**That rule lives *inside* the shared policy rather than beside it.** `known::choose` takes an
`adopts` flag. This tool and `km-admin` pass `false`; the offline remote passes `true`. The two
answers that would point a program at a machine nobody named become `Stay`.

Expressing *may not adopt* by not calling the policy at all means writing a second follow beside
`choose`, and eventually a third. That is precisely the drift one policy function exists to remove.
A hand-written follow drifts the same way twice. First, it follows a remembered id whenever the id
appears at a different address. Second, it misses rule 2: stay put when the address in hand is
still answering and reports the *same* id. That is one machine with two addresses, and the one in
hand is still working.

**The flag answers only what is left of that question.**
[`A machine is known by its id, and its address is a cache`](api-and-network.md#a-machine-is-known-by-its-id-and-its-address-is-a-cache)
refuses a different machine to every device that already knows one. So what `adopts` separates is a
program that knows of none. A remote opens on whatever is in the room; this tool waits to be
pointed at something. A workspace with a record never reaches the flag at all, and one without a
record starts on loopback.

**Following an id is not adopting, and the flag does not gate it.** That is this entry's own
exception, said once more because it is what makes the flag safe: the machine somebody chose, at a
new address. So is *remembering* — an address this tool wrote down is one it was told.

**One exception, and it is the same choice at a new address rather than a new one.** The tool
re-points itself when the machine id it is already pointed at reappears somewhere else. That is the
machine somebody chose. A router moving a lease overnight is not a decision anybody made. Refusing
to follow it would leave the tool installing into nothing while the machine it was told about sat two
meters away announcing itself.

**An identity that cannot change is what makes the guarantee strong.**
An address rule can promise only that the tool has not *looked*. This can say the machine is the
same machine.

Two guards keep the flag honest:

- the record's identity is written only from a `/discover` that answered **at the address in force**
- a record written by hand — the Settings box, a discovery row — carries none

So an address somebody named cannot inherit the previous machine's identity and be dragged off
after it.

**The browse is instant.** A registry that has been listening since the tool opened has already heard
from both machines in a two-machine house. So there is nothing to wait for, and the list is more
complete than any timeout made it.

It differs from the offline remote's discovery deliberately. **That one may re-point itself**
because the worst case there is browsing the wrong catalog. This one may not, because the worst
case here writes files somewhere nobody chose. **A row does not say whether a machine wants a
password.** Every machine wants one, so the label would carry no information. This tool can
send one too, so it would not be a warning either.

**The address in each row is the machine's own answer**, read from the `url` TXT record it advertises
— see `The advert names the address the machine chose` in
[`api-and-network.md`](api-and-network.md). Re-deriving it here cannot reach that answer: the ranking
that produces it reads interface names, and an announcement carries none. A row naming an address the
machine's own screen is refusing to show is a particularly bad failure for *this* button. Pressing
the second one points a tool that installs packages at it.

## Where the package builder keeps the machine it was told about

**In the workspace database, as one record, and the record is the shared `Known`.**

**The workspace is authoritative**, for the reason already in force: a `.kmbuild` is a document — see
[`A corpus is a document`](#a-corpus-is-a-document). A second computer opening the same corpus
should reach the same machine. An identity kept anywhere else would be free to disagree with the
address it describes.

**One record, not three rows.** `app_url`, `app_machine_id` and `app_machine_name` would have to be
written together and read together, which is a record with the type taken off it. What the type buys
beyond tidiness is `last_connected`, and the same shape as `km-remote`'s and `km-admin`'s files. So
the three programs share a *shape* as well as a policy.

**A workspace that has never been told takes this computer's last choice.** Without that a brand-new
`.kmbuild` starts at `http://127.0.0.1:8177` however many times its owner has pointed this tool
somewhere else. So the first install goes to a loopback address with nothing on it.

**That is not the adopting the entry above forbids, and the distinction is the whole of it.** The
per-user record is written *only* from an address somebody set by hand — the Settings
box or a discovery row they pressed. It is never written from a browse or from what answered.
Seeding a new workspace from a choice the same person made on the same computer is not pointing this
tool at a machine nobody named. It never overrides: a workspace that has a record ignores it entirely,
so a corpus carried between computers keeps pointing where it was set.

**`--machine` wins for the run and is not written down**, in the workspace or in the per-user
record. A run started against a test machine must not replace the machine somebody normally uses.
The address on the command line is already the address for that run. It is `km-admin`'s rule —
see [`Finding the machine, and remembering which one it was`](distribution.md#finding-the-machine-and-remembering-which-one-it-was)
— for the same reason.

The pinned machine
takes its identity in memory when it answers, and follows nowhere. Saving the Settings form on that
same address changes nothing; choosing any other address there ends the pin and is saved like any
choice. A pin needs no folder, so `--machine` does not make a named folder open eagerly.

**What the six-hour clock buys here is a sentence, not a move.** `STALE_AFTER` tunes `choose`'s rule 2
for a program that knows whether its address is answering. This one holds no connection —
every request builds a fresh client. So `online: false` and `answering_id: None` are honest rather than
lazy.

What `last_connected` gives it is the Settings page saying *nothing has answered here for six
hours or more*. That is exactly the state a corpus carried to another house is in: the address in the
database is now somebody's printer. The identity does not match anything on this network, and without
the sentence that is a failure with no explanation.

**And no page waits on a machine.** Settings probes, because saying whether the machine is reachable
is that page's job; every other page reads what is written down and nothing else. A test draws seven
pages with no machine configured and nothing on the network, and times each one. The failure
this guards against is a page that works and is slow.

## What language the package builder speaks is a setting

**`locale` in this curator's own `settings.json`, beside the suggested tags.** A television belongs
to a room, and a phone to the person holding it. So one is a machine setting and the other a cookie
the viewer carries. This belongs to whoever is curating, at one desk, in front of one corpus. That is
the machine's shape rather than the remote's, and the picker is on the Settings page for the same
reason the suggested tags are.

**`Accept-Language` is what an unset setting means.** A curator whose browser asks for Portuguese
gets Portuguese on the first page, with no clicks. That matters here more than anywhere, because a
language picker is the one control somebody who needs it cannot read the page around.

The browser's
answer is held in memory and never written. What goes in the file is a choice somebody made.
Persisting a header would turn whichever browser opened the tool first into a decision nobody took.
A tag no catalog answers to reads as unset rather than as an error, a preferences file being
something anybody can edit.

**Choosing answers `HX-Refresh`**, alone among this tool's controls. Every word on the document
changes:

- the nav
- the counts strip
- `<html lang>`
- the sentences `static/ui.js` reads off `<body>`

So no `hx-target` would be right, and the redraw is the confirmation. It arrives in the
language just chosen, which is what somebody who picked the wrong one needs.

**The panel's heading says which of two languages it means.** This tool has the other one on the same
screen:

- the browse bar filters by the language a song is *sung* in
- a package carries a default one
- `/songs/language-bulk` writes it over a whole filter

A key says `settings-locale-` for the
interface and `songs-language-` for the song, and neither word crosses. See
[`The interface has a locale; a song has a language`](foundations.md#the-interface-has-a-locale-a-song-has-a-language).

**A song's language is data and stays English.** `km_kmpkg::Language` carries an English name for the
pickers that have to show it, and the singer's remote already shows it that way. A catalog holding
the eighty-odd names would be translating the corpus rather than the tool. A tag somebody typed, a
folder's path, a song's title and a package's id are the same kind of thing.

**Zero takes the plural, and Portuguese needs saying so.** CLDR files 0 under `one` for Portuguese,
which is right for `0,5 dia` and wrong for a count: `0 favorita` is not what anybody writes. Every
plural in `pt-BR.ftl` carries an explicit `[0]` variant, and `zero_reads_as_a_plural` is what says a
new one has to as well.

**What is still English, and why each one is.** The tray and the menu bar are `km-tray`'s, shared by
three programs and given a locale by none of them. `--help` is clap's, built from doc comments on a
`Cli` struct, and
[`What a user reads is written in plain application language`](foundations.md#what-a-user-reads-is-written-in-plain-application-language)
governs it directly. The log is a diagnostic read beside a stack trace.

That is why every error type
here keeps its English `Display` and grows a `say` beside it. The `.kmbuild` shell verb `register.rs`
writes is Explorer's, in the language Windows is in. And the duplicate reason `dupes.rs` stores is
written and never read back.

**The window a failure before the page shows reads the setting directly.** It runs before the server
that would have read it, which is the one place the file is opened twice — see
[`A corpus that will not open is a page, and a failure before the page is a window`](#a-corpus-that-will-not-open-is-a-page-and-a-failure-before-the-page-is-a-window).

**Sorting does not follow the reader.** `remove_diacritics`, `COLLATE NOCASE`, `songs.sort_key` and
the A–Z strip are facts about the corpus. So a Portuguese reader gets the corpus in the order the
corpus is in. The number separator does not follow either: nothing here groups a number.

## Backing up what a person typed, and nothing else

**A backup carries the hand-set columns of `songs`, the favorites and their membership, and nothing
a scan can work out again.** The database keeps detection and correction in separate columns.
`det_title` is what the file said, and every scan rewrites it. `title` is what somebody typed, and no
scan touches it. So the split a backup needs is already drawn.

The rebuildable half is most of the index. It comes back by pointing the tool at the folder
again. The irreplaceable half is a few thousand rows out of a whole corpus, which is small
enough to be a file. Writing both would make the backup a copy of the database. A copy of a live
WAL database comes back inconsistent and says nothing about it.

**JSON rather than the YAML a `.kmspec` uses**, because nobody edits this one. A program reads it
at the moment somebody has lost their work. The argument that makes a description YAML
is that a person opens it.

For the same reason it does not `deny_unknown_fields`, where a description
does. A description is hand-written, and a typo in one silently dropped four thousand songs. A
backup refused by an older build is a recovery that did not happen. A newer `format` is *reported*
and read as far as this build understands it. An older one is refused with both numbers, by
[`A store opens at its current version or is refused`](foundations.md#a-store-opens-at-its-current-version-or-is-refused):
nothing in this build reads its shape.

**Songs rejoin by the content hash of their bytes, and one the corpus no longer holds is listed rather
than created.** The hash is already the song's identity, so it is the one key that survives a rebuilt
index, another machine, or a replaced drive. Inventing the missing rows was rejected, on the grounds
`A song with no title` was decided on.

`kind`, `duration_ms` and `warnings` are `NOT NULL`, with no
honest value for a song nobody has read. The invented row would enter the search index. It would show
in the browse list as a song with no file, no length and a fabricated suitability. A restore
rejoins; the answer to an unmatched song is to scan, and the tool says so.

**Packages and duplicate verdicts are deliberately not in it**, because a built package re-opens
through Import. The file says so in its own opening line, rather than leaving somebody to infer it
from an absence.

**A field the backup does not mention is never blanked, and the choice between filling blanks and
overwriting is made per run.** NULL in a hand-set column means "nobody has said" rather than "empty".
So a column the file is silent about is left exactly as it was under *both* settings. That is what
makes restoring the wrong file noisy rather than destructive. It is also why only the overwrite
direction sits behind a confirmation: filling blanks cannot take work away.

**An artist recorded as blank is a decision and travels as one.** It is the one hand-set column where
`''` and NULL differ. So *Title from file name*'s empty artist round-trips as `""`, and the `coalesce`
on either side of the switch reads it as a value. Overwriting writes the blank; filling blanks
leaves a blank already there alone. The confirmation on the overwrite direction is what covers it,
which is the same protection a title retyped since the backup has.

The choice is per *field* rather than per song, because a song is nine decisions and not one. A title
retyped since the backup survives, while the empty notes beside it are filled. Every value this
build will not take is found before anything is written, and reported as a line:

- a language that is not a code
- a rating above ten
- a transposition that will not fit
- a merge that would chain

One `UPDATE` per song means a single bad value would otherwise take the good one beside it down
with the statement.

**The default name carries the moment the backup was taken**, so a second backup sits beside the
first instead of destroying it: `km-package-builder-20260909T140233Z.kmbackup.json`. A backup is a
thing somebody takes more than once. The state anybody wants to go back to is the one before
whatever they have just noticed. Under a single fixed name, that is precisely the file the newest
backup has already overwritten. That is the failure the atomic write exists to prevent, reached by a
door the rename cannot cover.

The favorites backup the singer's remote hands out is dated for the
same reason, and to the day. This one goes to the second, because a curation session takes several in
an afternoon.

**The stamp is sliced off the moment already inside the document**, rather than read from a second
clock. So the name and the contents cannot disagree about when a backup was taken. Its separators
come out because `:` is not a character a Windows file name may hold. That leaves the form every
other dated file in this project uses — and that form sorts. So *newest* is a comparison of names
rather than a reading of the filesystem's clocks.

**`.kmbackup.json` is not the part that moves.** The scan collects by extension, and this is none of
the ones it collects. That is what `Where the tool's own output goes` leans on when it says nothing
has to be taught to skip the data folder. And a `.kmbuild`-shaped name in a folder that already holds
one stops the corpus opening at all. Only the stem may grow.

**The restore box suggests the newest**, because the moment that makes each backup its own file is
also what makes its name unmemorable. The alternative is somebody opening a file manager at the
moment they have already lost something. It is a suggestion and never a filled-in value. Restoring in
the overwrite direction is the one direction that takes work away. A real path one click from
*Restore* is a sharper edge than the confirmation covers. Only names this tool would itself have
written are offered, since a hand-named backup carries no moment to be newest by.

**The field list is written down, and a test makes the schema the authority over it.** A hand-written
list is how somebody's corrections get lost — one forgotten column and every typed title is gone, with
nothing to say so. A backup has nothing to intersect against, a JSON key not being a column. So the
guard is a test: it partitions `pragma_table_info('songs')` into the hand-set list and the derived
one. It fails, naming the column, on any that is in neither. Adding a column to the schema and
forgetting the backup is a red test rather than a silent loss.

## A page on another site cannot press this tool's buttons

**Every state-changing request is refused unless it came from this tool.** `Sec-Fetch-Site` must say
`same-origin` (or `none`, which is somebody typing the address); where a browser sends `Origin`
instead, it must match `Host`. GET and HEAD are not gated — they change nothing, and gating them
would refuse an ordinary link. A request carrying neither header is allowed.

**Having no login is the right call, and this is what makes it one.** The tool binds loopback and
holds no password of its own. "No login" alone does more work than it can carry: every form here
posts `application/x-www-form-urlencoded`, a CORS *simple request*. So a browser sends it to
`127.0.0.1` from any origin without a preflight and without asking. Any page the person has open
could otherwise reach:

- `/quit`.
- `/packages/{id}/delete`.
- `/settings/restore`.
- the bulk language and tag writes over a whole corpus.
- `/settings/backup`, which writes a file to a path taken from the form.

**The machine's own surfaces decide this the same way**: `km-admin-pages` and `km-remote-pages` both
set their cookies `SameSite=Lax`. That *is* this refusal, spelled where a cookie makes it available.
This tool keeps no cookie to hang it on, so it is a header check instead. That is the same rule,
stated in the only place left to state it.

**A request with no fetch metadata is allowed, deliberately.** `curl`, a script, and the dev remote
all send neither header, and refusing them would break every command-line check for nothing. The
lever this exists to remove is somebody's *browser*, and a browser always sends them. A guard that
also refused honest scripting would be traded away the first time it was inconvenient.

Not a token, because a token needs somewhere to live and something to put it in every form. That is
state and template churn for a tool whose threat is one specific browser behaviour that one specific
header already answers.

## A curation database says which schema it is, and one this build does not know is refused

**`.kmbuild` carries `PRAGMA user_version`, and a build opens only the numbers from
`OLDEST_SCHEMA_VERSION` to `SCHEMA_VERSION`.** SQLite will happily hand back rows from a table with
columns the reading build has never heard of. So without a version, a database written by a newer
build opens *silently*. The tool then curates somebody's corpus while quietly ignoring whatever that
build added. A database below the floor is refused for the mirror-image reason: queries written
for one shape would read another.

**Every change of shape is a numbered step whose predecessor is known exactly.** The ladder runs from
the floor to the current number, one arm per version, and an open of a current database costs one
`PRAGMA`. Raising the floor retires the steps below it. The rule across every store is
[`A store opens at its current version or is refused`](foundations.md#a-store-opens-at-its-current-version-or-is-refused).

**An unstamped file is two different things**, told apart by whether it has a `songs` table. A
brand-new file has none yet. `schema.sql` runs after the version check, so the triggers it creates
find every column they name, and the file is stamped current. One that has a `songs` table carries
no number this build can place, and is refused.

**The refusal names both numbers**, and there is no automatic answer to it. The build that wrote an
older database opens it, and a build at least as new opens a newer one.

## A folder opening at startup does not also offer the folder picker

**While an open is running, the Open page shows only that it is running.** The Recent list, the
Browse button and the type-a-path box are hidden until it has ended.

**A tool starting up correctly must not look broken for ten seconds.** Reopening last time's folder
lands on this page, because there is no workspace yet and every route is sent here until there is.
That happens whenever nobody names one, the whole point of
[`Reopening the last folder`](#reopening-the-last-folder). Drawing the progress line *and*,
underneath it, the entire chooser puts the folder picker on screen for the whole of a large corpus's
open. That is the thing this program shows when it has **failed** to open something.

**It is not a race.** The job registers before `axum::serve` is reachable, so the slot is never
transiently empty and the page always knows an open is running. The state is right; what must not
happen is the template drawing two states at once.

**Hidden rather than not rendered**, because the fragment that reports progress swaps into its own
element and the chooser is a sibling. Both endings that are not a redirect — a failure, and a slot
that emptied with nothing to show — put it back by name. A page left with one red sentence and nothing
to act on would be a worse version of the thing being fixed.

**The header line is the same fault in the place read first.** *Choose a folder of karaoke files*
above a folder being opened is the same wrong message; it says what is happening instead.

**A folder named on the command line lands here too**, where nothing else about the run needs the
database first — see
[`A corpus that will not open is a page`](#a-corpus-that-will-not-open-is-a-page-and-a-failure-before-the-page-is-a-window).
A run that opens before the server answers draws no page at all. Requests queue in the accept
backlog until the songs list can be served.

## A corpus that will not open is a page, and a failure before the page is a window

**The Open page opens a corpus named by a double-click, not before it exists.** The failure that
matters is ordinary: a database written by a newer build, two databases in one folder, or a missing
index. Opening it before the server exists means reporting it by returning out of `main`.

A windowed executable has no console and null standard handles, so that report reaches nothing at
all. The process exits and the screen does not change, which is indistinguishable from the file
association being broken. Opened as a job, the same sentence lands in red with the chooser beside it,
and the chooser is the point. The person can pick another folder, which is the one thing a dialog
cannot offer.

**What defers is the registered command and nothing else, which is what makes it a rule.** Standard
handles are inherited whatever the subsystem. So `km-package-builder <folder>` typed at a prompt is a
windowed run with a good stderr, and somebody waits on the sentence and the exit status. Deferring
there would take the status away, and a script would stop seeing failures.

So the test is *nowhere to read a refusal*. Beside it are the flags that name a corpus they cannot
act on until it is open: `--init`, `--scan`, `--machine`, `--backup`, `--restore`, each typed.
Everything else refuses before it binds, and the rule the ordering exists for is unchanged. **A run
that will refuse checks before it binds; a run that will report on a page binds first, because the
page is the report.**

**A refusal made before any job could start is left where the page looks.** A refusal that never
touches a database happens through an early return. A page that already posted the folder renders
that return, but a fresh start screen cannot. So the refusal goes into the slot the page already
polls, as a job that has already finished. Without that the folder holding two databases is the
silence all over again.

**A failure before a server exists opens a window.** An address already taken and a path naming
neither a folder nor a `.kmbuild` happen while there is no page to put anything on. The window is
built from the window library this feature already carries, rather than the platform's message box.
That dialog is an `unsafe` call with no safe wrapper. It would be a second exception to
[`Unsafe code, once`](foundations.md#unsafe-code-once) in the builds that already link a browser
engine.

**Every startup failure is logged first, whatever the shape.** That is the half `--log-file` exists
for, and could not do while the reason was returned rather than written down.

**A second launch says something is already listening, and stops.** It does not hand the folder over
and does not raise the other window. A handoff needs a probe protocol and a way to reach across
processes, and the running window's own Open page already opens another folder. What is *not*
claimed is who is listening: anything can hold that port. So the sentence says what is true and
names the way out of each case.

**The same failures reach `km-remote` and `km-admin` unchanged**, both of which return a bind failure
into the same kind of windowless `main`. Neither is a document handler and neither has an Open page,
so the shape here does not transfer whole. What transfers is the rule, and it is recorded as a known
hole rather than stated as though all three followed it.

## A song's corrections are three states, and leaving the page alone is the third

**NULL means nobody has said, an empty list means somebody said none, and a list is a decision.** A
detector proposes the corrections a song plays with, and a person settles them. A column with only
two states would make the first two indistinguishable. So a song whose corrections had been
deliberately turned off would be treated exactly like a song nobody had ever opened. And the next
detector to learn something would turn them back on.

**Opening a song and pressing Save stores nothing.** A form that came back saying exactly what
detection proposed is somebody who changed nothing. Writing that down would pin the song to one
afternoon's detector for good. What is stored is a list that *differs* from the proposal. That makes
the common case leave the corrections still learning: a curator correcting a title on a song whose
corrections they never looked at.

**Corrections have one control, the Advanced table.** A select on Details re-voicing the melody
channel repeats what the table offers on every channel, the melody's row included. A second control
for one correction is a second place to look for what is in force. Details keeps what the song *is*;
Advanced is where somebody goes having decided the file itself is wrong. Which channel the
melody *is* belongs there too, and has a section of its own.

**The Advanced table is one row per channel, and every correction on it.** Each row holds:

- what the file plays there.
- how many notes.
- the tracks that feed it.
- what the melody detector made of it.
- the four corrections a channel can be given.

The drum channel takes a mute but no instrument, a program change there naming a kit.

**The melody column is evidence, not the confidence beside the badge.** Confidence folds in how far
the winner stood clear of the field, which is a property of the song. Evidence is a channel's own
signals, which is what can sit in a column beside every other channel's.

**The number is the mark.** A column of percentages read against each other already says which
channel makes the strongest case. So a second column naming it in words repeats the column beside it.
Above nine tenths the number is coloured, because there the evidence stops being a candidate to weigh
and becomes an answer to read. What the numbers cannot say is which channel detection *took*, and
that stays marked.

**A correction carrying a value is drawn as a select**, which is the one departure from a box per
correction. On or off is the whole of a mute and the whole of a suppressed bank select. An instrument
is one of a hundred and twenty-eight, and a checkbox cannot say which. The select's first option is
the file's own instrument and posts nothing. So a correction absent and a correction refused stay the
same shape they are everywhere else.

**The table sends the whole list, through a route of its own.** The details form writes the title,
the artist and the language from what came back. So a corrections save posting into it would clear
all three.

**A correction this build cannot read is carried from the stored list.** No control can offer one and
no posted value can spell one. So a save that took only what came back would delete a correction
written by a newer build the first time somebody opened the page.

**A correction that waits for a person is marked suggested, and drawn unticked.** Detection splits
what it finds in two:

- what may apply itself, which is ticked and marked detected.
- what must be agreed to, which is neither ticked nor stored until somebody ticks it.

Marking both *detected* would put an unticked box beside a word that reads as a correction already
in force. The stuck bend is the correction of this kind. Like the bank select, it is offered only on
a channel where something proposes it or it is in force.

**A fix already in force is offered even where nothing proposes it**, which is what makes one
removable. A mute on a channel that has since stopped sounding would otherwise be invisible, and
removable only by somebody who already knew it was there.

**Detection runs when the page is drawn, not when the corpus is scanned.** A scanned column would
freeze one detector into every row of a corpus that nobody is going to read again. Re-scanning
a corpus of that size to pick up a new rule is not a thing anybody does. Parsing one file to draw one
song's page costs milliseconds.

**Re-opening a package takes back only a hand-edited list.** A package's detected corrections are this
build's own detector speaking through a file. Adopting them as decisions would put words in the
mouth of whoever built it.

**The preview carries what a curator typed, and not only the file.** `POST /debug/play-file` and
`POST /debug/play-upload` both take the title, the performer, the key, the corrections and the melody
channel beside the song. Every one of those lives in this database and nowhere in the bytes. So a
machine left to work them out for itself plays the song as it was *found*. But a curator who has just
done one of the following is looking at their own page and listening to a different song:

- silenced a channel.
- retyped a title.
- dropped the key two semitones.
- named the melody a detector abstained on.

Absent and empty are distinct on the wire for the reason they are distinct in the column. The melody
channel is absent to detect, `null` (`none` in a form) to say there is none, and a number to name
one.

**They travel as one thing rather than as five parameters**, because the list is the kind that grows.
Each is a value the file does not know. And a preview that answered four of five questions would be
a preview somebody stopped trusting.

## The melody channel is chosen in the column that weighs it

**The Advanced table is where a melody channel is named, and a song can be said to have none.** A
detector that abstains as ambiguous leaves a singer no guide melody at all. The machine offers that
toggle only on a song that has a channel for it. One that picks the wrong part offers a toggle that
silences the wrong line. Both are judgements a person makes by looking at what every channel plays,
which is this table and nowhere else.

**In the melody column, not beside it.** The evidence and the choice are the same question. A page
that showed the numbers here and took the answer somewhere else would ask somebody to hold a table in
their head. They would do that while looking at a select.

**A radio per channel, and *no melody* in the column head.** One song has one melody channel, so the
controls are one group and choosing any of them unsays the rest. *No melody* is the one answer that
is about the song rather than about a channel. So it has no row to sit in, and sits in the head. The
drum channel gets no radio: it sounds a kit rather than a part.

**Three states, stored the way a song's corrections are.** Three answers exist:

- NULL, meaning nobody has said, so detection stands.
- a channel, which is a decision.
- *none*, which is a decision too.

That third answer is what a column of sixteen-or-nothing could not express. A song deliberately marked
as having no melody would otherwise be indistinguishable from one nobody had opened.

Picking the channel already shown stores nothing. That is both what an untouched page must leave
behind and the whole of *hand this back to the detector*. There is no separate clear button, because
choosing what detection found **is** clearing it.

**Beside what detection found, never over it.** The scanned column goes on recording what the file
implies, so a rescan keeps learning and an answer outlives it. The row that says which channel
detection settled on therefore stays marked where it was, even when somebody has chosen another. The
two are different claims, and the disagreement is the thing the table exists to show.

**It reaches the package, and a rebuild keeps it.** The built manifest carries the chosen channel and
marks the field as edited. That is what makes a rebuild from source keep a person's answer instead of
the detector's. *No melody* is the case that needs the marker most. The field is absent both when
detection abstained and when somebody said there is none, and only the marker tells those apart. A
channel somebody named carries a signal saying so rather than a confidence, because confidence is the
detector's unit and a judgement has none.

**A chosen channel is drawn whether or not it sounds.** The table already shows a channel carrying a
correction that has stopped sounding. That is on the grounds that a mute nobody can see is a mute
nobody can take off. A choice nobody can see is one nobody can take back.

## A scan shows its steps, what is left, and can be stopped from the page

**The Scan page lists every step of a run before the run takes it.** Each step is done with how long
it took, running with a clock that keeps counting, waiting, or not needed this time. A forced pass
over a whole corpus is hours, and most of that is one step. The rest are single passes with nothing
inside them to count. A name for the running step alone says neither what is left. Nor does it say
whether a step that has shown the same sentence for five minutes is still working.

**A step that runs only when a row changed says so while it waits.** Grouping duplicates, indexing
folders and measuring for the query planner run only when the run wrote or removed a row. Nobody
knows which until the reading is over. Listing them as certain promises work an unchanged rescan
never does; leaving them off hides the longest tail a forced pass has.

**A run that reached its end is drawn in green**: its phase, its bar and its ticks. A reading at
100% still has the tail to come, and a stopped or failed run has ended too. So neither the bar nor
the word *finished* alone says the work is done.

**The walk counts the files it has found.** It has no total to draw a bar against, and on a spinning
disk it is minutes.

**The reading step shows its rate and the time left, from the last half minute.** An incremental scan
skips its unchanged files in a burst and then reads the changed ones far slower. So an average since
the start promises an end that keeps moving away. Nothing is shown for the first ten seconds of
reading, because an estimate from less misleads.

**A Stop button asks the run to stop, the way Ctrl-C does.** The run writes what it has already read
and draws none of a finished run's conclusions. The press answers at once, and the panel says
*stopping* until the run has written its last batch. That is because a request that waited would hold
the page for as long as that takes.

**Stop reaches the steps after the reading too.** Grouping duplicates, indexing folders and measuring
are each skipped once Stop is pressed, and indexing ends part-way through. A stop there keeps the
scan. Every file was read and the folder is marked as scanned, so the panel does not call the scan
partial. The folder tree is rebuilt on the next visit to the Folders page.

## How many files a scan reads at once

**One for each processor.** A scan reads files that many at a time and writes what they hold through
a single writer. The number reaches the Scan page's buttons as much as `--scan` and `--reanalyze`.

**The count does not change how fast a scan runs, and that is measured rather than assumed.** Over a
real corpus on a spinning disk, one reader and twenty-four are the same speed. That is to within less
than the noise between two runs at the same count. A scan's disk traffic is the writer's index
maintenance. The files being read are a twenty-fifth of it, so dividing that twenty-fifth differently
cannot move the total. The table is in
[the architecture note](../architecture/package-builder.md#scanning).

**Settable anyway, because that is a fact about one disk and not about disks.** `--jobs` holds for a
single run, and `KM_SCAN_JOBS` says the same where a shortcut has nowhere to put a flag. `scan_jobs`
in the settings file is where somebody who has measured their own disk keeps the answer. What holds
for one run outranks what is kept. It sits there rather than in the corpus folder,
because a disk is a fact about the box. So it should follow a curator from one folder to the next.

**A value naming no number leaves the processor count standing rather than ending the run.** It is
read on the way into a scan somebody has just asked for. Taking that scan away from them to report a
stale variable in a shortcut costs more than starting it.
