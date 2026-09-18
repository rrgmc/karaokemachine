# The owner's page

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## One crate, and the hosts that serve it

`crates/machine/km-admin-pages` is server-rendered askama templates with **no JavaScript at all**,
compiled into whichever binary links it. **Two hosts serve it**: the machine in its own process, and
`km-admin` over HTTP. Without that,
[`Two admin surfaces, one vocabulary`](../decisions/distribution.md#two-admin-surfaces-one-vocabulary)
is a rule holding two implementations of one settings pane, one output picker and one
factory-password banner in step by hand.

**The difference between hosts is a set of `dyn` traits rather than a mode enum**, which is
`km-remote-pages`' arrangement one layer over.

| | The machine, at `/admin/` | `km-admin`, on a desktop |
|---|---|---|
| Reaches the machine | in process, `km_api::ops::*` and the trait objects | HTTP against `/api/v1` |
| Refused packages | listed, with the folder each was found in | **absent** — see below |
| Searching for pictures and banks | never | its whole reason for existing |
| Packages, rotation, installed banks | listed and managed | the same, minus a package's size, which the API does not publish |
| Screen language | both | both, over `PUT /admin/machine/locale` |
| Password reset | yes | no: a reset draws a PIN on a television this program is not beside |
| Which machine, and the password for it | no question: it is one | its own front door, outside this crate |
| The guard | denies by default; the page faces a LAN | loopback only, no password of its own — and so nothing here authorizes its writes |

### The traits are grouped by tab, and that is deliberate

One trait per tab — `Machine`, `Switches`, `Sound`, `Songs`, `Pictures`, `Uploads`, `Problems` —
because **the tabs *are* the vocabulary decision**: a person who has learned one surface reads the
other by its tab strip, so a seam shaped the same way can be read against it. `km-remote-pages` groups
by *capability* instead, which is right there, where its two modes differ by feature, and wrong here,
where they differ by which errand a program is for.

`Admin::over` takes one host and fans it out behind all of them. Passing the same `Arc` seven times
read as though the seven might differ; they do not, and a host that could answer them from different
places — one talking to two machines — is not a thing this product has.

### Every method is `async` even though one host answers from memory

**So that *how to get off the runtime* is the implementation's decision rather than the page's.** The
in-process host reads a catalog behind a mutex an install holds for **seconds**, and
`karaokemachine/src/remote.rs` is the written account of ignoring that: *"every page of this remote
would sit on a worker waiting, and with as many workers as the box has cores, the API, the pages and
the event stream stop together."* A synchronous trait would force that call up into a crate with no
idea which reads are cheap. `Songs::packages` is the method that most needs it; `Problems::refused`
is the one that deliberately does **not** hop, being a clone out of its own mutex and read on every
page for the nav badge.

## How the second host is wired

`tools/cmd/assets/km-admin` mounts `km_admin_pages::router` at **`/admin`** — the same prefix the
machine nests it under — and merges its own routes there beside it. `/` redirects to `/admin/connect`,
and **temporarily**, because a browser keeps a permanent redirect and follows it without asking the
server again: where the front door is would stop being the program's to decide. `Bound::front_door`
is the other half of that rule — a window and a browser are opened on the door itself, never on the
origin — and `views::CONNECT_PAGE` is the one spelling all three share.

**The prefix is not a choice.** The shared templates write their links out (`/admin/machine`,
`/admin/sound?all=1`), so the alternative was a `base` field on the chrome prefixed at every link and
every redirect: forty handlers and every template threading a value that is empty in one host and
`/admin` in the other. Mounting at the same prefix is one line, and that program is loopback-only with
no bookmarks in the world to break.

**What that costs is a prefix spelled by hand in every `src`, `action`, `hx-get` and `Location`, and
`own_paths_are_routes_this_program_mounts` is what keeps them true.** It sweeps every root-relative
path in km-admin's own templates, plus the consts that reach markup through a struct field, and
drives each through the router under a method nothing mounts — `405` says the path is there, `404`
says it is not, and no handler runs either way. A dropped prefix is invisible without it: an `img`
falls back to its alt text and a redirect lands on a page that is simply not there.

**One path had to move.** `/sound/{id}/remove` was claimed by both routers — *remove this bank from
the machine* in one, *remove the copy on this computer* in the other — and axum panics on the overlap
rather than picking, which is the right way for that to surface. km-admin's three row controls are
under `/sound/fetch/{id}/…` now, which also reads better: they belong to the fetching page.

### A page is a dozen requests on the second host, and that is what shapes the reads

**Every trait method the chrome and the Machine tab call is a field access in the machine's own
process and an HTTP request in `km-admin`** — the asymmetry `Every method is async even though one
host answers from memory` exists for. What it costs is not visible in either host's own code: the
Machine tab's reads come to eleven requests, `Switches::read` alone being four, and a machine that
is not answering charges the ask timeout for each.

Four rules keep that from being a page that looks broken.

**The one page that must draw with the machine off makes no read at all.** `Admin::door` is the
exception to everything below rather than a fourth case of it: a host's front door is where somebody
goes because nothing is answering, so it is drawn from what the host already has on disk. See
`Admin::door, and why it makes no trait call`.

**Independent asks go together.** `Switches::read`'s four are `try_join!`ed, and so are the chrome's
name, factory-password and problem-count reads and the Machine tab's identity-and-switches pair.
None of them is an input to another, so in sequence they buy latency and nothing else.

**One read per page, not one per reader.** The Machine tab wants the demo pair that the chrome does
not carry, so it reads the switches and hands them to `chrome_from` rather than building a chrome
that reads them again.

**`km-admin`'s client remembers the last moment.** A `/discover` answer is reused for a moment,
because the heading, the information panel and the factory-password banner each ask for one and none
can see the others; and a machine that failed to answer is assumed to still be failing for a few
seconds, so the page after this one does not pay to find that out twice. **A page's asks honour that
assumption and a person's do not** — what somebody presses is always tried, or a machine that has
just come back could not be reached by the press that would prove it. The memo rides on the client,
so choosing a machine replaces it.

### Two states, two routers, merged

The searching, the jobs and the fragments need km-admin's own `State`; its *pages* also need the
`Admin` the shared router was built from, because that is what wraps a body in the shared chrome. So
there are two routers, each finalized with its own state, merged under the one prefix — `server::Pages`
is the second. One state carrying both would have every job endpoint reaching past a field it has no
use for.

### `Admin::shell`, and why the seam is a wrapper

**askama cannot `{% extends %}` across a crate**: it resolves a template path against the *including*
crate's `templates/`. km-admin has two pages this crate does not and must not have — its front door,
and the searching that `A fourth program, rather than a fourth tab on the owner's page` keeps off the
machine — and both need the shared strip, heading and factory-password banner.

So the chrome is a wrapper: the host renders its own body and `Admin::shell` puts it inside
`shell.html`, which extends the same `layout.html` every page here does. `Tab::Home` exists for the
marking and this crate draws no page under it.

**`Shell::content` is `|safe`, and the contract is: render a template, pass its output, never build
the string.** What arrives is markup a host wrote, in the same sense any `{% block content %}` here is
markup this crate wrote. A request's value formatted into it would be an injection nothing in this
crate could catch.

**The notice is a parameter of both wrappers, because the banner is chrome.** A refusal answers with
a redirect carrying `?kind=&said=`, and its destination is as often a host's own page as one of this
crate's, so a host has to be able to hand a notice over. Drawing one inside the body instead puts a
second banner in a second place, which is the drift the wrapper exists to delete.

### `Admin::door`, and why it makes no trait call

**The page it draws is the page somebody is on before there is a machine**, so every part of the
chrome is blank or wrong: the heading is the machine's name, the banner warns about the machine's
factory password, the badge counts its problems, and the strip's entries all lead to tabs that can do
nothing yet. `Chrome::door` fills the four fields the `<head>` needs and asks nobody anything;
`layout.html` gates the strip on `chrome.front_door`.

**The measured half is the one to keep.** `handlers::chrome` `try_join!`s the name, the
factory-password flag and the problem count, and over HTTP a machine that is not answering charges
the ask timeout for each — on the one page a person reaches *because* the machine is off. So the door
costs one render and no request, and the three-second browse arrives afterwards as a fragment of the
host's own.

**`/admin/connect` is the third route this crate names and does not own.** The two the `searching`
capability links to are the others, and the rule is the same: a host that sets `choose_machine` must
serve it, because `handlers::refusal` sends a write refused for want of a password there.

### One stylesheet, and the one thing a host may put in the head

`km-admin` kept a 669-line stylesheet and its own `layout.html` until the second host landed, and
**the layout was the only thing linking either it or htmx**. Deleting that file was the whole point
of the change; the `<link>` and two `<script>` tags went with it and nothing failed. Every test here
drives a router and asserts on markup, so a *missing* tag is invisible to all of them: the searching
still ran, still finished and still wrote its files, and `_job.html` simply never replaced itself
again. A bank download reported the first second of itself for nine hours.

Both halves are now the seam's rather than a template's.

**The stylesheet is one file**, `static/admin.css`, and its last section is what survived of the
other one — the searching's panels, provider cards, review grid, job bar and toasts, rewritten in
this file's variables. `--ground`, `--panel`, `--edge` and `--radius` were the same four ideas under
other names, which is most of why two files could drift: nothing made them disagree, and nothing
made them agree either. The rest of that file styled a header bar, a settings strip, a
discovered-machines list and a factory-password banner this chrome now draws, and went with the
templates that used them. `One stylesheet, and the tool wears the machine's amber` is the decision,
including why the provisional magenta was dropped rather than gated.

Merging the files fixed three things nobody was aiming at, and all three are the same shape — a rule
one surface had and the other did not. `.mono` was defined only in `km-admin`'s sheet and is used by
this crate's own *Different machine* pane, so the machine's page had unstyled markup. Bare links had
a floor rule there and none here, so a confirmation's Cancel drew in the browser's blue on a
near-black ground. And `.hint` and `.note` were two names for one look in two files, which is now one
declaration and two names, because they are genuinely two uses.

**`Admin::scripts` is how a host asks for a script, and the machine asks for none.** The list is on
`Chrome`, so it is emitted by `layout.html` and reaches every page that host serves rather than only
the pages the host writes itself. `no_page_the_machine_serves_carries_a_script` is what holds the
other side: the machine declares none, the loop draws nothing, and its pages are byte-for-byte
scriptless.

**The tabs are where it earns that reach.** Songs, Pictures and Sound each carry a multipart upload
form, a package runs to two gibibytes, and the forward to a machine is allowed an hour. A form post
that reports nothing while it runs cannot be told from a program that has stopped, and a script is
the only thing that can say a navigation is in flight. What that gives up is the stronger claim that
the tabs drawn here are script-free on *whichever* host serves them; `No htmx, no script at all`
asks only that the machine's be, and it still is.

**The tray a script puts its messages in is emitted under the same condition, from the same field.**
htmx does not swap a non-2xx response, so `ui.js` is what words a refusal, and it looks its tray up
by id and returns when the element is absent — a download reporting its first second for an hour,
indistinguishable from a server that never answered. One field decides both the scripts and the
tray, so a host that declares none grows no element nothing there could fill. It sits outside
`<main>`: a fixed live region announces what just happened, where the notice banner says it in
place. It also carries the sentence an upload's indicator says, because a catalog string has to
reach a script somehow and this is already the one element gated on there being one.

**A missing element is the failure a file scan cannot see**, which is the argument above about a
missing `<script>`, and it is the same failure twice —
`this_programs_own_pages_load_what_they_need` asks for the tray on a **rendered** page for that
reason. `km-package-builder`'s `views::toast_only` and its `hx-swap-oob="afterbegin"` wrapper are the
shape to copy if a message ever has to come *from* a handler rather than from a failed request.

The test that guarded scriptlessness was a file scan, and a file scan is exactly what could not have
caught this. It is two tests now: the scan stays for an `hx-` attribute a template might gain, and
`no_page_the_machine_serves_carries_a_script` reads seven rendered pages, which is the only place a
tag that is absent can be seen. `this_programs_own_pages_load_what_they_need` asks the other
direction on km-admin's side — is the sheet linked, are the scripts loaded, is what they point at
actually served.

### The guard is one mechanism on the machine and none on a tool
`Capabilities::gate_every_route` is true on the machine and false in `km-admin`, and both halves are
argued where the flag is defined. **The middleware is the only caller of `Guard::allows`**, so that
flag chooses between one mechanism and none rather than between two: with it off, nothing in this
crate authorizes any write.

On a tool `allows` answers *do we hold a token* — a fact about that program's session, not about the
browser making the request — and the authority on whether a write may happen is the machine, which
refuses an untokened call with a 401 that arrives as `AdminError::Unauthorized` and is worded here.
Per-handler checks would be this crate second-guessing that, and could refuse a write the machine
would have taken.

**What asks `allows` on a tool is the Machine tab, once per load, and it is a read rather than a
gate.** A tool has to be *given* the machine's password before any write it makes can land, so the
page needs to know whether the program holds a token: the answer chooses between drawing a password
box and drawing the sentence saying there is nothing left to type. Nothing is refused by it, which is
what keeps the paragraph above true.

`Guard::remembering` and `Guard::forget_password` are the box beside the field, and both default to
doing nothing so the machine's own host implements neither. `Guard::login` takes what was ticked
rather than a second call taking it afterwards, because logging in is the one moment the password is
in hand: a host keeps the *token* it buys, never the password, so a tick applied later would have
nothing to store. `RemoteMachine::writing` is where a remembered one is spent — on the calls that
need a token and not on the public reads beside them, `Switches::read` alone being four concurrent
reads that would otherwise be four concurrent logins.

`a_refused_guard_stops_a_write_on_the_machine_and_the_machine_decides_for_a_tool` asserts the
asymmetry: the machine's write must not happen, and a tool's must reach the machine. **A test over one
host cannot state that**, which is the argument for running the shared tests under both.

### What each host's capabilities say

`Capabilities::machine()` has everything on but one. `Capabilities::desktop()` turns off the Problems
tab and the password *reset*; it turns on `choose_machine` and the two doors to the searching.

**`screen_language` is on for both**, on the `installed_*` flags' footing: `GET /locale` and
`PUT /admin/machine/locale` give a host over HTTP the same answer the machine has in process. What
that pane sets is the *television's* language, never the page's — `km-admin`'s own is chosen on its
front door and lives in a cookie, and the two headings are worded apart because confusing them is
the likeliest misreading of either.

**`choose_machine` is the one flag the machine's own page does not have**, and the asymmetry is which
side of the door the page is on. It draws two things and neither of them is a control: the strip's
way back to the host's front door, and the sentence on the Machine tab saying whether this program
holds a token. The machine has no say over which machine it is and no session with itself, so its
page draws neither. `AdminError::wants_password` is what carries a refusal to that door from whichever
tab it was made on.

**The three `installed_*` flags are on for both**, which is `What it is not is a second /admin/`: one
page set draws those controls, so a host rendering them is not a second place to keep right.

**A flag alone draws nothing.** Every one of those controls needs a real answer from `Songs`, `Sound`
or `Pictures` — eleven trait methods over HTTP, seven `Call` variants swept against
`km_api::routes::SURFACE`, three DTO conversions and a percent-encoder. A flag turned on over stubs
produces a table with no rows and controls that refuse.

**The encoder is what to read before adding a call.** Six of those calls put an id in their path, and
an id is a file name or a manifest's slug rather than anything this program chose. Escaping has to
happen while the id is still a separate value: build the path with `format!` and then push its
`/`-separated pieces through `Url`, and an id's own slashes are already separators —
`RemoveBank("../../admin/password")` addresses `/api/v1/admin/audio/soundfonts/admin/password`. So
`Call::path` puts the id through `one_segment`, and because unreserved characters pass through
unchanged, `SURFACE`'s own sample ids compare equal and the sweep still matches a whole built path
against a whole declared one.
**Two values cross lossily, both under one published rule** — *the page prints the sentence, a remote
gets the flag*. A bank's and a picture's `why_not_removable` is a boolean on the wire, because the
machine's own sentence names a path, so this host words a vaguer one itself; and a package's size does
not cross at all, `PackageDto` carrying no byte count, so that column is empty on a tool and holds a
number on the machine's own page.

`Chrome::program` is the one thing the shared layout says on one host and not the other:
`What the tool calls itself` requires km-admin's pages to say *KaraokeMachine Admin*, and a layout
showing only the machine's name would have dropped that. It was caught by a test asserting the
product's name was on the page.

## What the seam absorbed

Each of these was something the page had to know and now does not — and each is a real difference
between the two hosts rather than tidiness:

- **Knowing which machine answered is a write nobody asked for.** `/discover` is read for the
  heading, the panel and the factory-password banner, and what it says about the machine's *id* is
  recorded on the way past: the follow keys on that id, and so does a remembered password, so a host
  that read the answer and threw the identity away left a machine that moved unrecognizable and
  nothing to key a password under. The machine's own host records nothing, having never been anywhere
  else.
- **A rename is two writes.** `Controller::set_machine_name` writes `settings.json` and
  `ApiState::set_machine_name` moves the running value that answers `/discover`. A rename reaching
  one and not the other is the bug found on the appliance in 2026-08: the machine came back under its
  old name after a power cut. A host over HTTP sends one `PUT`.
- **A password is three**, plus argon2 — and, for a reset, generating the PIN the television will draw.
- **`set_demo_delay` answers with what the machine stored**, not what was asked for, because the route
  caps it. A page echoing the request would tell somebody their 9999 had been saved.
- **`set_output` answers with the device list**, because the confirmation names the device that was
  *chosen* and `active_name` is what is *sounding*. On an idle machine nothing is — the endpoint goes
  back five seconds after the last song — so that message once read *"The sound comes out of not yet
  opened now."*

## `AdminError`, and the two variants that carry no code

`words.rs` predicted this: *"There is no error vocabulary here…nothing arrives from across a wire
needing a code to be rendered from."* A second host makes that false, so faults travel as **codes**
that the page renders in the reader's language — `km-remote-core`'s arrangement, for its reason: the
host that finds out about a fault may be talking to a machine in another language, and the page doing
the rendering is the one that knows what its reader speaks.

**Two variants are deliberately not codes.** `Refused` carries the machine's own sentence and is shown
as it arrived, because the machine is the authority on what a name may be and a second opinion here
would be a second thing to keep in agreement with it. `Busy` carries one too, and is kept apart from
`Refused` because **the page says something different about it**: choosing an output while a song is
loaded answers *busy*, which is `warn` — "try again when the song ends" — where *there is no such
device* is `bad`. One function, `severity`, decides which; that distinction previously lived in a
`match` inside one handler and had no test, so moving the operation behind a trait could have
flattened it silently.

`ERROR_KEYS` is **listed rather than scanned**, which is the one place this crate departs from
`words`' habit: that scanner finds `msg("…")` calls in one file, and these are a `match`, so all of
them would have been reported as messages nothing asks for. `every_error_key_is_listed` asserts the
list and the `match` agree in both directions, which is what stops the exemption hiding the drift it
exists to allow.

## Two things that did not become traits

**There is no `Faults` trait.** Every row on the Problems tab that is not a refused package is
something another tab already knows — the bank complaint and the output fallback are `Sound`'s, the
empty rotation is `Pictures`'. A trait of its own would have been a second way to ask the same
questions, which is what this seam exists to stop. So `handlers::faults` composes from the traits
already there.

**`Problems` is the only `Option`**, and for a privacy decision rather than a capability: the refused
list identifies its rows by a *path on the machine*, and `PackageProblemDto` drops it on purpose —
*"the directory layout of the machine under the television is nobody's business but the owner's."* The
page may print it because it runs inside the machine and already prints paths in a refusal's sentence;
the rule is *the page prints the sentence, a remote gets the flag*. So a host over HTTP has no way to
answer that tab, and `None` says so. An empty list would be a claim, and a false one — `refused_rows`
draws no rows at all rather than *Every package was loaded*. The same shape `km-remote-pages` gives
`Favorites`.

## Where the in-process implementation lives, and why not in the host

`in_process::ThisMachine` is in **this crate**, not in `karaokemachine`, which departs from
`km-remote-pages` — whose online implementation is in `karaokemachine/src/remote.rs`, with
hand-written stubs in its own tests.

The reason it *can* be here: the whole implementation is a `km_api::ApiState`, which this crate
already depends on. The reason it *must*: these page tests exist to prove that pressing a button
changes the machine — `nested_with_state` hands the state back so a test can ask what it ended up
with — and a stub would answer canned values and prove nothing of the sort. The three alternatives
are a `testing` feature exporting fakes, stubs that prove nothing, and a third copy of a hundred and
fifty lines in a test file. **A third copy of the thing this seam exists to delete is not a trade
worth making.**

What the seam still buys is unchanged, and is the property to check before adding a handler:
**nothing in `handlers` or `views` names `ApiState`**, so this crate does not depend on a machine
being in its own process — and the compiler holds that rather than a convention, there being no field
left to reach through.

## Which types cross the seam unchanged, and the one that does not

`AudioOutputs`, `SoundFontBanks`, `SoundFontStatus`, `WallpaperState` and `Picture` are `km_api`'s own
and cross as they are. Both hosts can already name them — `km-admin` takes `km-api` for exactly this,
stated in its manifest: *"The machine's DTOs and its mDNS discovery, so the shapes this talks to are
stated once. It carries no HTTP client of its own, which is what makes it safe to take from here."* A
parallel set of row types here would be one more thing to keep in step with the machine's, on a page
whose whole job is to show what the machine says.

**`Listing` is the exception, and the reason is the workspace boundary.**
`km_catalog::InstalledPackage` would mean **rusqlite**, and the second host is a desktop program with
no catalog of its own reaching across an `exclude` line under a rule that keeps what it takes small.
Naming `AudioOutputs` costs nothing; a SQLite dependency for a handful of fields is a different
bargain. So `Listing` holds the ones the page reads, and one more is a compile error at both hosts —
which is the right way round to find out it is wanted.

`views`' own row types stay, and they earn their place: they hold rendered sizes, composed sentences
and resolved labels, which are answers about a *page* rather than about a machine.
