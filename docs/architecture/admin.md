# The owner's page

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## One crate, and the hosts that serve it

`crates/machine/km-admin-pages` is server-rendered askama templates with **no JavaScript at all**. It
compiles into whichever binary links it. **Two hosts serve it**: the machine in its own process, and
`km-admin` over HTTP. Without that,
[`Two admin surfaces, one vocabulary`](../decisions/distribution.md#two-admin-surfaces-one-vocabulary)
would be a rule holding two implementations in step by hand. Each would have its own settings pane,
output picker and factory-password banner.

**The difference between hosts is a set of `dyn` traits rather than a mode enum.** That is
`km-remote-pages`' arrangement, one layer over.

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

There is one trait per tab: `Machine`, `Switches`, `Sound`, `Songs`, `Pictures`, `Uploads` and
`Problems`. The reason is that **the tabs *are* the vocabulary decision**. A person who has learned
one surface reads the other by its tab strip, so a seam of the same shape reads against it.

`km-remote-pages` groups by *capability* instead. That is right there, where its two modes differ by
feature. It is wrong here, where they differ by which errand a program is for.

`Admin::over` takes one host and fans it out behind all of them. Passing the same `Arc` seven times
read as though the seven might differ, and they do not. A host that could answer them from different
places, such as one talking to two machines, is not a thing this product has.

### Every method is `async` even though one host answers from memory

**So that *how to get off the runtime* is the implementation's decision rather than the page's.** The
in-process host reads a catalog behind a mutex, and an install holds that mutex for **seconds**.
`karaokemachine/src/remote.rs` is the written account of ignoring that:

> *"every page of this remote
> would sit on a worker waiting, and with as many workers as the box has cores, the API, the pages and
> the event stream stop together."*

A synchronous trait would force that call up into a crate with no idea which reads are cheap.
`Songs::packages` is the method that most needs it. `Problems::refused` is the one that deliberately
does **not** hop. It is a clone out of its own mutex, and every page reads it for the nav badge.

## How the second host is wired

`tools/cmd/assets/km-admin` mounts `km_admin_pages::router` at **`/admin`**, the same prefix that the
machine nests it under. It merges its own routes there beside it. `/` redirects to `/admin/connect`,
and **temporarily**. A browser keeps a permanent redirect and follows it without asking the server
again. The location of the front door would then stop being the program's to decide.

`Bound::front_door` is the other half of that rule: a window and a browser open on the door itself,
never on the origin. `views::CONNECT_PAGE` is the one spelling all three share.

**The prefix is not a choice.** The shared templates write their links out (`/admin/machine`,
`/admin/sound?all=1`). So the alternative was a `base` field on the chrome, prefixed at every link and
every redirect. Forty handlers and every template would thread a value that is empty in one host and
`/admin` in the other. Mounting at the same prefix is one line. That program is loopback-only, with no
bookmarks in the world to break.

**What that costs is a prefix spelled by hand in every `src`, `action`, `hx-get` and `Location`, and
`own_paths_are_routes_this_program_mounts` is what keeps them true.** It sweeps every root-relative
path in km-admin's own templates. It also sweeps the consts that reach markup through a struct field.
It drives each path through the router under a method nothing mounts. A `405` says the path is there,
a `404` says it is not, and no handler runs either way.

Without the test, a dropped prefix is invisible. An `img` falls back to its alt text, and a redirect lands on a page that is not there.

**One path had to move.** Both routers claimed `/sound/{id}/remove`. In one it meant *remove this bank
from the machine*, and in the other *remove the copy on this computer*. axum panics on the overlap
rather than picking, which is the right way for that to surface. km-admin's three row controls are
under `/sound/fetch/{id}/…` now. That also reads better, because they belong to the fetching page.

### A page is a dozen requests on the second host, and that is what shapes the reads

**Every trait method the chrome and the Machine tab call is a field access in the machine's own
process and an HTTP request in `km-admin`.** That asymmetry is what `Every method is async even though
one host answers from memory` exists for. Neither host's own code shows what it costs. The Machine
tab's reads come to eleven requests, and `Switches::read` alone is four. A machine that is not
answering charges the ask timeout for each.

Four rules keep that from being a page that looks broken.

**The one page that must draw with the machine off makes no read at all.** `Admin::door` is the
exception to everything below, rather than a fourth case of it. A host's front door is where somebody
goes because nothing is answering. So the host draws it from what it already has on disk. See
`Admin::door, and why it makes no trait call`.

**Independent asks go together.** `Switches::read`'s four are `try_join!`ed. So are the chrome's name,
factory-password and problem-count reads, and the Machine tab's identity-and-switches pair. None of
them is an input to another, so in sequence they buy latency and nothing else.

**One read per page, not one per reader.** The Machine tab wants the demo pair that the chrome does
not carry. So it reads the switches and hands them to `chrome_from`, rather than building a chrome
that reads them again.

**`km-admin`'s client remembers the last moment.** It reuses a `/discover` answer for a moment. The
heading, the information panel and the factory-password banner each ask for one, and none can see the
others. The client also assumes that a machine that failed to answer is still failing for a few
seconds. So the page after this one does not pay to find that out twice.

**A page's asks honour that assumption and a person's do not.** The client always tries what somebody
presses. Otherwise a machine that has just come back could not be reached by the press that would
prove it. The memo rides on the client, so choosing a machine replaces it.

### Two states, two routers, merged

The searching, the jobs and the fragments need km-admin's own `State`. Its *pages* also need the
`Admin` that the shared router was built from, because that is what wraps a body in the shared chrome.
So there are two routers, each finalized with its own state, merged under the one prefix.
`server::Pages` is the second. One state carrying both would have every job endpoint reaching past a
field it has no use for.

### `Admin::shell`, and why the seam is a wrapper

**askama cannot `{% extends %}` across a crate**: it resolves a template path against the *including*
crate's `templates/`. km-admin has two pages that this crate does not and must not have. One is its
front door. The other is the searching that `A fourth program, rather than a fourth tab on the
owner's page` keeps off the machine. Both need the shared strip, heading and factory-password banner.

So the chrome is a wrapper. The host renders its own body, and `Admin::shell` puts it inside
`shell.html`, which extends the same `layout.html` that every page here does. `Tab::Home` exists for
the marking, and this crate draws no page under it.

**`Shell::content` is `|safe`, and the contract is: render a template, pass its output, never build
the string.** What arrives is markup a host wrote, in the same sense that any `{% block content %}`
here is markup this crate wrote. A request's value formatted into it would be an injection that
nothing in this crate could catch.

**The notice is a parameter of both wrappers, because the banner is chrome.** A refusal answers with
a redirect carrying `?kind=&said=`. Its destination is as often a host's own page as one of this
crate's. So a host has to be able to hand a notice over. Drawing one inside the body instead puts a
second banner in a second place. That is the drift the wrapper exists to delete.

### `Admin::door`, and why it makes no trait call

**The page it draws is the page somebody is on before there is a machine**, so every part of the
chrome is blank or wrong. The heading is the machine's name. The banner warns about the machine's
factory password. The badge counts its problems, and the strip's entries all lead to tabs that can
do nothing yet. `Chrome::door` fills the four fields that the `<head>` needs and asks nobody anything.
`layout.html` gates the strip on `chrome.front_door`.

**The measured half is the one to keep.** `handlers::chrome` `try_join!`s the name, the
factory-password flag and the problem count. Over HTTP, a machine that is not answering charges the
ask timeout for each. That would happen on the one page a person reaches *because* the machine is off.
So the door costs one render and no request. The three-second browse arrives afterwards as a fragment
of the host's own.

**`/admin/connect` is the third route this crate names and does not own.** The two that the
`searching` capability links to are the others, and the rule is the same. A host that sets
`choose_machine` must serve it, because `handlers::refusal` sends a write refused for want of a
password there.

### One stylesheet, and the one thing a host may put in the head

`km-admin` kept a 669-line stylesheet and its own `layout.html` until the second host landed. **The
layout was the only thing linking either it or htmx.** Deleting that file was the whole point of the
change. The `<link>` and two `<script>` tags went with it, and nothing failed.

Every test here drives a router and asserts on markup, so a *missing* tag is invisible to all of
them. The searching still
ran, still finished and still wrote its files, and `_job.html` simply never replaced itself again. A
bank download reported the first second of itself for nine hours.

Both halves are now the seam's rather than a template's.

**The stylesheet is one file**, `static/admin.css`. Its last section is what survived of the other
one. That is the searching's panels, provider cards, review grid, job bar and toasts, rewritten in
this file's variables. `--ground`, `--panel`, `--edge` and `--radius` were the same four ideas under other
names. That is most of why two files could drift: nothing made them disagree, and nothing made them
agree either.

The rest of that file styled a header bar, a settings strip, a discovered-machines list
and a factory-password banner. This chrome now draws those, so that styling went with the templates
that used it.

`One stylesheet, and the tool wears the machine's amber` is the decision. It includes why the
provisional magenta was dropped rather than gated.

Merging the files fixed three things nobody was aiming at. All three are the same shape: a rule that
one surface had and the other did not.

- `.mono` was defined only in `km-admin`'s sheet, and this crate's own *Different machine* pane uses
  it. So the machine's page had unstyled markup.
- Bare links had a floor rule there and none here. So a confirmation's Cancel drew in the browser's
  blue on a near-black ground.
- `.hint` and `.note` were two names for one look in two files. That is now one declaration and two
  names, because they are genuinely two uses.

**`Admin::scripts` is how a host asks for a script, and the machine asks for none.** The list is on
`Chrome`, so `layout.html` emits it. It reaches every page that host serves, rather than only the
pages the host writes itself. `no_page_the_machine_serves_carries_a_script` holds the other side.
The machine declares none, the loop draws nothing, and its pages are byte-for-byte scriptless.

**The tabs are where it earns that reach.** Songs, Pictures and Sound each carry a multipart upload
form. A package runs to two gibibytes, and the forward to a machine is allowed an hour. A form post
that reports nothing while it runs cannot be told from a program that has stopped. A script is the
only thing that can say a navigation is in flight.

What that gives up is a stronger claim: that the tabs drawn here are script-free on *whichever* host
serves them. `No htmx, no script at all` asks only that the machine's be, and it still is.

**The tray a script puts its messages in is emitted under the same condition, from the same field.**
htmx does not swap a non-2xx response, so `ui.js` is what words a refusal. It looks its tray up by id
and returns when the element is absent. The result is a download reporting its first second for an
hour, indistinguishable from a server that never answered. One field decides both the scripts and
the tray, so a host that declares none grows no element that nothing there could fill.

The tray sits outside `<main>`. A fixed live region announces what just happened, where the notice
banner says it in place. The tray also carries the sentence an upload's indicator says. A catalog
string has to reach a script somehow, and this is already the one element gated on there being one.

**A missing element is the failure a file scan cannot see.** That is the argument above about a
missing `<script>`, and it is the same failure twice. So
`this_programs_own_pages_load_what_they_need` asks for the tray on a **rendered** page.
A message may one day have to come *from* a handler rather than from a failed request. For that,
copy the shape of `km-package-builder`'s `views::toast_only` and its `hx-swap-oob="afterbegin"`
wrapper.

The test that guarded scriptlessness was a file scan, and a file scan could not have caught this. It
is two tests now. The scan stays for an `hx-` attribute that a template might gain.
`no_page_the_machine_serves_carries_a_script` reads seven rendered pages, which is the only place a
tag that is absent can be seen. `this_programs_own_pages_load_what_they_need` asks the other direction
on km-admin's side: is the sheet linked, are the scripts loaded, and is what they point at actually
served.

### The guard is one mechanism on the machine and none on a tool
`Capabilities::gate_every_route` is true on the machine and false in `km-admin`, and both halves are
argued where the flag is defined. **The middleware is the only caller of `Guard::allows`.** So that
flag chooses between one mechanism and none, rather than between two. With it off, nothing in this
crate authorizes any write.

On a tool, `allows` answers *do we hold a token*. That is a fact about that program's session, not
about the browser making the request. The authority on whether a write may happen is the machine. It
refuses an untokened call with a 401, which arrives as `AdminError::Unauthorized` and is worded here.
Per-handler checks would be this crate second-guessing that, and they could refuse a write the
machine would have taken.

**What asks `allows` on a tool is the Machine tab, once per load, and it is a read rather than a
gate.** A tool has to be *given* the machine's password before any write it makes can land. So the
page needs to know whether the program holds a token. The answer chooses between drawing a password
box and drawing the sentence saying there is nothing left to type. It refuses nothing, which keeps
the paragraph above true.

`Guard::remembering` and `Guard::forget_password` are the box beside the field. Both default to doing
nothing, so the machine's own host implements neither. `Guard::login` takes what was ticked, rather
than a second call taking it afterwards. Logging in is the one moment the password is in hand. A host
keeps the *token* it buys, never the password, so a tick applied later would have nothing to store.

`RemoteMachine::writing` is where a remembered one is spent. It spends it on the calls that need a
token, and not on the public reads beside them. `Switches::read` alone is four concurrent reads that
would otherwise be four concurrent logins.

`a_refused_guard_stops_a_write_on_the_machine_and_the_machine_decides_for_a_tool` asserts the
asymmetry: the machine's write must not happen, and a tool's must reach the machine. **A test over one
host cannot state that**, which is the argument for running the shared tests under both.

### What each host's capabilities say

`Capabilities::machine()` has everything on but one. `Capabilities::desktop()` turns off the Problems
tab and the password *reset*. It turns on `choose_machine` and the two doors to the searching.

**`screen_language` is on for both**, on the `installed_*` flags' footing. `GET /locale` and
`PUT /admin/machine/locale` give a host over HTTP the same answer that the machine has in process.
That pane sets the *television's* language, never the page's. `km-admin`'s own language is chosen on
its front door and lives in a cookie. The two headings are worded apart, because confusing them is
the likeliest misreading of either.

**`choose_machine` is the one flag the machine's own page does not have**, and the asymmetry is which
side of the door the page is on. It draws two things, and neither of them is a control. One is the
strip's way back to the host's front door. The other is the sentence on the Machine tab saying whether
this program holds a token. The machine has no say over which machine it is and no session with
itself, so its page draws neither. `AdminError::wants_password` carries a refusal to that door from
whichever tab it was made on.

**The three `installed_*` flags are on for both**, which is `What it is not is a second /admin/`. One
page set draws those controls, so a host rendering them is not a second place to keep right.

**A flag alone draws nothing.** Every one of those controls needs a real answer from `Songs`, `Sound`
or `Pictures`. That takes eleven trait methods over HTTP and seven `Call` variants swept against
`km_api::routes::SURFACE`. It also takes three DTO conversions and a percent-encoder. A flag turned on
over stubs produces a table with no rows and controls that refuse.

**The encoder is what to read before adding a call.** Six of those calls put an id in their path. An
id is a file name or a manifest's slug, rather than anything this program chose. Escaping has to
happen while the id is still a separate value.

Suppose a call builds the path with `format!` and then pushes its `/`-separated pieces through `Url`.
Then an id's own slashes are already separators:
`RemoveBank("../../admin/password")` addresses `/api/v1/admin/audio/soundfonts/admin/password`. So
`Call::path` puts the id through `one_segment`. Unreserved characters pass through `one_segment`
unchanged. So `SURFACE`'s own sample ids compare
equal, and the sweep still matches a whole built path against a whole declared one.

**Two values cross lossily, both under one published rule**: *the page prints the sentence, a remote
gets the flag*. A bank's and a picture's `why_not_removable` is a boolean on the wire, because the
machine's own sentence names a path. So this host words a vaguer one itself. A package's size does
not cross at all, because `PackageDto` carries no byte count. So that column is empty on a tool and
holds a number on the machine's own page.

`Chrome::program` is the one thing the shared layout says on one host and not the other.
`What the tool calls itself` requires km-admin's pages to say *KaraokeMachine Admin*. A layout showing
only the machine's name would have dropped that. A test asserting the product's name was on the page
caught it.

## What the seam absorbed

Each of these was something the page had to know and now does not. Each is a real difference between
the two hosts, rather than tidiness:

- **Knowing which machine answered is a write nobody asked for.** The page reads `/discover` for the
  heading, the panel and the factory-password banner. The host records the machine's *id* from
  it on the way past, because the follow and a remembered password both key on that id.
  A host that threw the identity away would not recognize a machine that moved, and it would have
  nothing to key a password under. The machine's own host records nothing, having never been
  anywhere else.
- **A rename is two writes.** `Controller::set_machine_name` writes `settings.json`, and
  `ApiState::set_machine_name` moves the running value that answers `/discover`. A rename reaching one
  and not the other is the bug found on the appliance in 2026-08. The machine came back under its old
  name after a power cut. A host over HTTP sends one `PUT`.
- **A password is three**, plus argon2. For a reset, it also generates the PIN the television will
  draw.
- **`set_demo_delay` answers with what the machine stored**, not what was asked for, because the route
  caps it. A page echoing the request would tell somebody their 9999 had been saved.
- **`set_output` answers with the device list.** The confirmation names the device that was *chosen*,
  and `active_name` is what is *sounding*. On an idle machine nothing is, because the endpoint goes
  back five seconds after the last song. So that message once read *"The sound comes out of not yet
  opened now."*

## `AdminError`, and the two variants that carry no code

`words.rs` predicted this: *"There is no error vocabulary here…nothing arrives from across a wire
needing a code to be rendered from."* A second host makes that false. So faults travel as **codes**
that the page renders in the reader's language. That is `km-remote-core`'s arrangement, for its
reason. The host that finds out about a fault may be talking to a machine in another language. The
page doing the rendering is the one that knows what its reader speaks.

**Two variants are deliberately not codes.** `Refused` carries the machine's own sentence, and the
page shows it as it arrived. The machine is the authority on what a name may be. A second opinion
here would be a second thing to keep in agreement with it.

`Busy` carries one too. It is kept apart from `Refused` because **the page says something different
about it**. Choosing an output while a
song is loaded answers *busy*, which is `warn`: "try again when the song ends". *There is no such
device* is `bad`.

One function, `severity`, decides which. That distinction previously lived in a `match` inside one
handler and had no test. So moving the operation behind a trait could have flattened it silently.

`ERROR_KEYS` is **listed rather than scanned**, which is the one place this crate departs from
`words`' habit. That scanner finds `msg("…")` calls in one file, and these are a `match`. So it would
have reported all of them as messages nothing asks for. `every_error_key_is_listed` asserts that the
list and the `match` agree in both directions. That stops the exemption hiding the drift it exists to
allow.

## Two things that did not become traits

**There is no `Faults` trait.** Every row on the Problems tab that is not a refused package is
something another tab already knows. The bank complaint and the output fallback are `Sound`'s, and
the empty rotation is `Pictures`'. A trait of its own would be a second way to ask the same questions,
which is what this seam exists to stop. So `handlers::faults` composes from the traits already there.

**`Problems` is the only `Option`**, and for a privacy decision rather than a capability. The refused
list identifies its rows by a *path on the machine*, and `PackageProblemDto` drops it on purpose:

> *"the directory layout of the machine under the television is nobody's business but the owner's."*

The page may print it, because it runs inside the machine and already prints paths in a refusal's
sentence. The rule is *the page prints the sentence, a remote gets the flag*. So a host over HTTP has
no way to answer that tab, and `None` says so. An empty list would be a claim, and a false one.
`refused_rows` draws no rows at all, rather than *Every package was loaded*. `km-remote-pages` gives
`Favorites` the same shape.

## Where the in-process implementation lives, and why not in the host

`in_process::ThisMachine` is in **this crate**, not in `karaokemachine`. That departs from
`km-remote-pages`, whose online implementation is in `karaokemachine/src/remote.rs`, with hand-written
stubs in its own tests.

The reason it *can* be here: the whole implementation is a `km_api::ApiState`, which this crate
already depends on. The reason it *must*: these page tests exist to prove that pressing a button
changes the machine. `nested_with_state` hands the state back, so a test can ask what it ended up
with. A stub would answer canned values and prove nothing of the sort.

There are three alternatives. One is a `testing` feature exporting fakes, and one is stubs that prove
nothing. The third is a copy of a hundred and fifty lines in a test file. **A third copy of the thing this seam exists to
delete is not a trade worth making.**

What the seam still buys is unchanged, and it is the property to check before adding a handler.
**Nothing in `handlers` or `views` names `ApiState`**, so this crate does not depend on a machine
being in its own process. The compiler holds that, rather than a convention, because no field is left
to reach through.

## Which types cross the seam unchanged, and the one that does not

`AudioOutputs`, `SoundFontBanks`, `SoundFontStatus`, `WallpaperState` and `Picture` are `km_api`'s own
and cross as they are. Both hosts can already name them. `km-admin` takes `km-api` for exactly this,
and its manifest states it:

> *"The machine's DTOs and its mDNS discovery, so the shapes this talks to are
> stated once. It carries no HTTP client of its own, which is what makes it safe to take from here."*

A parallel set of row types here would be one more thing to keep in step with the machine's. It would
sit on a page whose whole job is to show what the machine says.

**`Listing` is the exception, and the reason is the workspace boundary.**
`km_catalog::InstalledPackage` would mean **rusqlite**. The second host is a desktop program with no
catalog of its own. It would reach across an `exclude` line, under a rule that keeps what it takes
small.

Naming `AudioOutputs` costs nothing, but a SQLite dependency for a handful of fields is a
different bargain. So `Listing` holds the ones the page reads, and one more is a compile error at both
hosts. That is the right way round to find out it is wanted.

`views`' own row types stay, and they earn their place. They hold rendered sizes, composed sentences
and resolved labels, which are answers about a *page* rather than about a machine.
