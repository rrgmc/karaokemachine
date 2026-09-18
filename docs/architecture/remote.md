# The remotes

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## The dev remote

`tools/dev/remote/` is one dependency-free page touching every route in the API surface, including
the ones a product surface would hide: the debugging switch, `debug/play-file`, install and
uninstall, and a raw WebSocket log. **Its purpose is not to be a
good remote; it is to prove the surface is complete** before a real client exists. The melody toggle
is *disabled and explained* when detection abstained rather than hidden, because that is the behavior
a real remote has to get right.

**It is also the caller `GET /audio/soundfonts?all=true` exists for.** That parameter widens `offers`
from the nine banks the machine offers to the whole sixty-odd-row survey, each row marked `offered`
or not; the page opens on the shortlist and passes it only when asked. (It was written to be the
line between `/dev/` and the singer's Setup tab; that tab has gone, so both sides of it are now the
one page.) It is a width and not a
permission — `audio.read` at either, and `POST /audio/soundfont/fetch` was never gated by rank — so
what the shortlist governs is what somebody is *shown without asking*. The page asks. A download
reports through the same list rather than through an event, so the page polls it every two seconds
while one is running and not otherwise: a byte count moving four times a second is the last thing
that belongs on a stream every phone in the room is reading.

**One of its controls is for a route it does not assume the machine has.** `DELETE
/audio/soundfonts/{id}` is drawn on every installed row but the bundled one, and a machine without
that route answers `unknown_endpoint` — which lands in the transcript and the panel's note like any
other refusal. That is the right failure for a test harness: the page's job is to say what the
surface is, and a control that reports a missing route is doing that job rather than failing at it.

**The plural in that path is load-bearing.** `/audio/soundfont` is the singleton — `GET` reports the
bank in force and `PUT` changes it — while `/audio/soundfonts` is the collection, so a member delete
belongs on the second, the way `DELETE /packages/{id}` sits under `/packages`. The practical half is
sharper than the aesthetic one: on the singular, `{id}` would sit beside the static
`/audio/soundfont/fetch`, and a static segment wins the match — so a bank whose filename slugged to
`fetch` could never be deleted, and would answer 405 from the wrong route while doing it.

**Seek is absolute in the API and relative in the page.** `POST /transport/seek` takes a position and
nothing else, which is the right shape: a machine that accepted "forward 30 seconds" would have to
agree with the caller about where *now* is, and across a network they cannot. So the buttons read the
state first, add the delta and post the result — one loopback round trip, and the difference between
landing where somebody meant and landing wherever the last event said.

**It is desktop-only, and that is a decision rather than a gap.** A phone reaches it and drives the
machine, and the layout is cramped there. Making the page *fit* a phone would not make it *right* for
one, because what it puts on screen is not what a singer wants in their hand.

**The layout is one CSS grid and a widget is a `<section>` in it**, with `section.wide` spanning the
row for the four that carry tables. The five groups are the same trick read the other way: a
full-row `h2.group` cannot share a row, so a heading both names the widgets under it and forces the
break above them. **There is no wrapper element, and that is the point** — a `<div>` per group would
have to re-create the grid inside itself, and each group would then size its columns against its own
width, so a two-widget group would draw wider cards than a three-widget one. It also means the
grouping costs the JavaScript nothing: every widget is still reached by `getElementById`, so moving
one between groups is a move of markup and no more.

Compiled into the binary in **every** build, so no build can ship without it, and served only when
somebody asks — `--dev-remote` for a run, `api.serve_dev_remote: true` for good. It reaches no route
the API does not already expose to the same caller, so this withholds a page rather than a permission;
what it is for is not publishing a developer's console on the LAN by default now that `/` and
`/admin/` answer what an owner needs. See `Dev remote in release builds` in
[`../decisions/remotes.md`](../decisions/remotes.md).

## One crate, two modes

`km-remote-pages` is server-rendered askama templates with htmx, one SSE stream per browser, compiled
into whichever binary links it. **The difference between the modes is a `Capabilities` struct rather
than a mode enum**, so a feature moves between them by editing one line in Rust rather than the
markup.

| | Online — served by the machine at `/` | Offline — `km-remote` |
|---|---|---|
| Catalog | the library, in process | its own mirror, downloaded from a machine |
| Playback | in process | HTTP + a WebSocket to a machine, which may be away |
| Favorites, A–Z picker | absent | present |
| Setup tab | the name, the language and a link to `/admin/`; a Packages tab with two or more packages | the name, the language, the machine card and the backup; the same Packages tab |
| Song book, owner's page | linked — the machine serves both | absent: neither is this process's to serve |

**Every page is in both modes, including Setup.** `Capabilities` gates what is *on* a page and never
whether the page exists, which is why the online Setup tab is two preference rows rather than a
404 — the name and the language are about whoever is holding the phone, and neither mode is a mode
where that is not a question.

**Two capabilities are on in the online mode and off in the other**, which is the direction that
reads backwards until you see why: `song_book` and `owner_page` are both links to something *the
machine* serves and this crate does not, so they are same-origin where these pages run inside it and
point at nothing where they do not. The owner's-page link is also the only thing in this product that
points at `/admin/` at all — see
[`The singer's remote links to it`](../decisions/api-and-network.md#the-singers-remote-links-to-it-at-the-foot-of-the-setup-tab).

Four traits are the seam: `Songs` (the catalog, which answers with the machine powered down),
`Machine` (playback, which can be *absent*), `Favorites` (with no online implementation at all) and
`Connect` (which machine this device talks to). They are `async` and used as `dyn`, which is the whole
reason `async-trait` is a dependency: the mode is a compile-time fact per binary, but making forty
handlers generic over it would buy nothing.

**A hidden package reaches every catalog query as a list of ids.** `BrowseQuery::hidden_packages`
and the `hidden` argument of `Songs::artists`, `languages` and `tags` come from the `km_hidden`
cookie through `Prefs::read`, and never from a link. Both catalogs turn the list into
`AND package_id NOT IN (…)`: `km_catalog::SearchQuery::exclude_packages` and the `Library` pickers
online, `Mirror::conditions` and the mirror's pickers offline. `tags` joins `song_tags` to `songs`
only when the list is not empty. `Songs::song` and `Songs::resolve` take no list, which is what keeps
a typed number and a favorites folder reaching a hidden song. `Songs::packages` names the packages for
`/setup/packages`. See
[`A person hides a package from their own song list`](../decisions/remotes.md#a-person-hides-a-package-from-their-own-song-list).

`Connect` is the one whose implementation could not possibly live in this crate — a locator, a mirror
and a data directory are all the core's, and the online mode has none of them to lend. What it cost
the pages crate is a struct, a trait and a builder step; what it bought is that the Android and iOS
shells got the card with no edit at all, **which is the second-caller proof the seam existed for.**

## The connection budget

**A browser allows about six connections to one host, and this remote spent more than that on a tab
press.** The symptom was requests hanging for about thirty seconds, which reads like a slow server
from every angle. It is not: neither handler touches the catalog or does any I/O. **The server never
saw the request for those thirty seconds.**

Three things spent the budget, none wrong on its own: the tabs are ordinary links, so every press
replaces the document; every document opened an `EventSource` and **never closed it**, so a page in
the back/forward cache went on holding a connection; and the static files carried a content type and
nothing else — no `Cache-Control`, no `ETag` — so all four were re-fetched on every navigation.

So a navigation wanted six connections of its own against a budget already short by however many
abandoned streams were open. **And `hx-sync="body:queue last"` then turned one queued request into a
dead page**, because that attribute allows the document one in-flight request at a time — so the first
thing to queue stopped every later tap. That attribute is right and stays; what it did here is what a
serializing queue does when the thing at the head cannot finish.

**A page nobody pressed for is queued like any other.** `Load more` fires itself when it is scrolled
to, so this queue has a producer that is a flick rather than a tap. A fetch arriving behind something
already in flight is queued, `last` discards it as soon as anything else queues, and htmx has stamped
that button as revealed by then, so it will not fire a second time. The list goes quiet until somebody
presses the button, which is what the button is drawn for. A scroll costs one connection at a time and
cannot outrun the budget on its own; what it can do is lose its turn silently.

The fix is the first two, not the third: the script closes its source on `pagehide` and reopens on
`pageshow`, and the static routes answer `immutable` with a `?v=` stamp computed from an FNV-1a of the
embedded bytes in a `const fn`, so it changes when and only when a file does.

- **`visibilitychange` is the wrong event to close on, and the right one to wake on.** Closing there
  would close on a window merely losing focus, and a queue left up on a second screen is a real use.
  But it is the *only* event a WebView gives on the way back — an Activity stopping is not a
  navigation, so `pagehide` and `pageshow` never fire — so `live.js` takes a fresh stream when the
  page becomes visible and still closes on nothing but a navigation. See `A page that comes back
  asks again` below.
- **`unload` is the wrong spelling.** Registering for it disqualifies the page from the back/forward
  cache in every current browser — it would free the connection by deleting the feature the connection
  is competing with.
- **Closing is only safe because the hub replays** — and reopening is what *makes* it replay, which
  is the half that matters more. The stream sends the latest of every state event as it opens, so a
  page coming back out of the cache is brought up to date by the mechanism that already fills one on
  its first load. Nothing is added for the reopen.

**A leaked stream is invisible unless something counts them.** The handler logs the listener count as
each one opens, so a number that climbs while somebody presses tabs is a line in a log rather than an
inference — and it needs no browser devtools to see:

| | streams open after each navigation |
|---|---|
| before | 1, 1, 2, 3, 4, 5 — one left behind by every press |
| after | 1, 1, 1, 1, 1, 1 |

**The structural answer is still open**: boosting the tab bar so a press swaps `<main>` rather than
replacing the document would leave exactly one stream per session.

## A page that comes back asks again

Reported from a phone: switching away from the Android remote and back **always** showed the red
strip, recovery took about ten seconds, and pressing a tab noticed it at once. Three separate things
had to be true for that, and only the third is about the banner.

**Nothing fired.** The Android shell's `onStop` calls `web.onPause()` and `web.pauseTimers()` and its
`onStart` the mirror image, with no reload and no JS evaluated. An Activity stopping is not a
navigation, so `pagehide` and `pageshow` never fire — the stream is neither closed nor reopened, and
because the reopen is what triggers the replay, **the page is never brought up to date at all.** It
keeps whatever fragment it was last pushed, and the pump will not correct it: `publish_connection` is
gated on the connection having *changed* since the previous tick.

**Pressing a tab is what made this look like something else.** A tab is an ordinary link, so a press
replaces the document and `chrome()` renders the banner from `machine.connection()` as it is now. The
display was stale, not the connection — which is why "it notices immediately if I switch tabs" is a
clue about the page and reads like a clue about the network.

**And the JS timers resume where they stopped.** `pauseTimers` is process-wide, so the eight-second
banner clock freezes on the way out and resumes with its full remaining time on the way back: a strip
armed just before a background is *guaranteed* to be on screen afterwards. Nothing about the machine
needs to be wrong for that.

The fix is one mechanism doing both halves. `live.js` takes a fresh stream when the page becomes
visible, which makes the hub replay every fragment; and `GET /events` treats a stream opening against
an absent machine as a request to try it now, which cuts short a backoff measured before the process
was frozen. It also disposes of the frozen-timer problem without a wall clock anywhere: the replay
delivers a *new* banner element, so the timers still pending fire on a detached node — the behavior
`animationend` on a removed badge already relies on.

**A floor of one reopen a second** (`REOPEN_MIN_GAP_MS`) is not tidiness. A restore from the
back/forward cache fires `pageshow` and `visibilitychange` in the same tick, and a notification shade
can flap visibility twice in a moment; without it each one aborts a request that had just gone out
and pokes the machine again.

**What cannot be tested from here**, and it is worth knowing before the next change: there is no
browser in `tests/pages.rs` and no Android build in CI, so the guard is an assertion on the served
text of `live.js` and the assumption that a Chromium WebView drives `document.visibilityState` from
window visibility. If a device ever shows it does not, `MainActivity.onStart` already documents itself
as "take the multicast lock, **and wake the page**" and already calls `evaluateJavascript` twice, so
the fallback is one line in a method whose comment claims to do this.

## Catching up sent one fragment of the several it owed

`Hub`'s broadcast channel holds 64 frames, and a subscriber that overruns it is not resent the frames
it missed — it is resent the current state, which is both cheaper and more nearly what it wanted.
**That branch handed over the first frame of the snapshot and dropped the rest.** `snapshot()` returns
the latest of each of the eight replayed events in `REPLAYED` order, `PLAYER` first, so a page that
fell behind was resynchronized with a player card and left holding a stale banner, a stale dot and a
stale queue.

Three things make that kind of fault survive:

- **`stream::unfold` yields one item per poll**, so returning several frames from one branch is not
  something the shape allows — and `.next()` on the iterator is what the code reaches for. The fix is
  a `VecDeque` in the unfold's state, drained a frame per poll before the receiver is read again.
- **Nothing corrects it afterwards.** The pump republishes the connection only when it *changes*
  (`CONNECTION_INTERVAL`, and a comparison against the previous tick), so a page that missed the
  transition never hears about it again. Only a navigation, which re-renders `chrome()` from
  `machine.connection()`, or a fresh stream, which replays, puts it right.
- **The one branch with a paragraph explaining why it mattered.** `CHANNEL_CAPACITY`'s doc names the
  case exactly — "a phone that locks its screen mid-song stops reading" — which is the phone this was
  reported from, and the comment was right about everything except what the code below it did.

**And an `Sse` is a response, not a stream**, so there was nowhere to assert any of this: the hub's
tests could reach `snapshot()` and the raw channel and not the thing built out of them. `stream()` is
now two functions, a private `frames()` returning the replay chained to the live feed and a wrapper
that maps it into `SseEvent`, which is the whole reason the resynchronization has tests at all.

## A stream nobody can leave

The remote holds one WebSocket to the machine, and the connection state every page draws from is a
by-product of it: `follow` sets `online` when the socket opens and the loop around it writes down the
reason when the socket ends. **Both halves assumed a socket that ends.**

Reported from a real house, and it needs no unreachable machine to reproduce. A remote following a
machine at one address was pointed at a second machine through the machine card. Everything worked
— browsing, queueing, the catalog refresh — because `point_at` swaps the `Api` every HTTP call
uses. The banner read `Connecting…` for the rest of the evening, and the Now tab went on showing the
**first** machine's playback, because the follower was still attached to the first socket and had no
way to be told. `Connecting…` is only ever replaced by the outcome of an attempt, and no attempt was
running.

Two mechanisms, and neither is a special case of the other:

- **The address is a `watch`, not a lock.** Re-reading it once per iteration means reading it *after*
  the current stream ends — and a stream to a machine that is still running never does. Every wait
  loses a `select!` to a redirect instead: the follow, the backoff sleep, and the wait for a first
  address. So *Use this* takes effect at once instead of after up to the thirty-second backoff, and
  the socket to the machine being left is dropped with the future rather than followed on in the
  background. `borrow_and_update` rather than a separate read, so the redirect that started an
  iteration does not immediately end it.
- **The read has a deadline, and the machine's own heartbeat is what makes that unambiguous.**
  `km_api::events::run_state_ticker` publishes a `state` event every `STATE_INTERVAL` (250 ms)
  whether or not anything is playing — its doc insists on the "whether", because an idle machine's
  remote still has to learn that the queue emptied. So twenty missed heartbeats is a machine that is
  gone, not one with nothing to say, and `STREAM_IDLE_TIMEOUT` is written as that multiplication so
  that a machine which ever stops ticking breaks the build here rather than leaving remotes flapping.
  Without it a box switched off at the wall — no FIN, no RST — leaves the read pending until the
  process exits, and the whole loop behind it.

Two smaller faults fell out of the same reading. `connect_async` had no timeout where every HTTP call
has had one since the first of them, which on a host that refuses by saying nothing is the other way
to sit in a single attempt indefinitely. And a polite close has to publish `offline` itself: left to
the next attempt failing, `online` stays true for a backoff's worth of seconds after the television
is switched off, with the dot green throughout.

`follow` takes its idle deadline as an argument rather than reading the constant. That is the one
thing here a test has to be able to shorten, and a paused clock cannot do it: `tokio` advances a
paused clock whenever the runtime is idle, which during a socket handshake it is — so the test would
race its own connection.

**And a third fault sat in the same twelve lines, found from a real evening rather than by reading.**
`backoff` outlives an iteration, and only the *polite close* arm reset it — so the doubling applied to
every other kind of drop, which is all of the ordinary ones: the idle deadline says "The karaoke
machine stopped answering." and a socket error says "The connection dropped (…)", and both are `Err`.
A machine that blinked repeatedly through an evening therefore walked the wait out to the
thirty-second cap **while every attempt was succeeding**, and each later blink cost up to half a
minute of "not reachable" for a box that was answering again within one.

What makes it hard to report is the shape rather than the size: the remote gets worse the longer it
is left running, and restarting it fixes it. And the intent was already written down — `The dev remote
comes back on its own` says "coming back to an address that is coming back is cheap and worth trying
often" — so this is the code disagreeing with its own decision rather than a policy anybody chose.
`next_backoff(current, had_connected)` is the whole of it, a pure function with three tests and no
socket, and `had_connected` has to be read **before** `set_reachability` overwrites it, because
`follow` setting reachability on the handshake is the only record that the attempt worked. Doubling is
for a machine that is *not there* — it is what stops a remote hammering an address nothing is
listening at — and it has no business measuring a connection that came up.

## A disclosure that survives a swap but not a navigation

The now bar's transport is hidden until the ▾ beside the title is pressed, and the whole of the
mechanism is one CSS rule and a checkbox:

```css
#nowbar-controls:not(:checked) ~ #nowbar .transport { display: none; }
```

The mark flips to ▴ when it is open, off the same checkbox, which is why it is two spans in the
label rather than a `::before` whose `content` swaps — a glyph in `content` is in neither the
accessibility tree nor a selection:

```css
#nowbar-controls:not(:checked) ~ #nowbar .controls-toggle .when-open,
#nowbar-controls:checked ~ #nowbar .controls-toggle .when-closed { display: none; }
```

The checkbox is in `queue.html`; the toggle is a `<label for>` inside `#nowbar`, class
`.controls-toggle`. That split is what gives the two lifetimes the behavior needs, and neither is
achievable by the obvious alternatives:

- **It must survive a republish.** `#nowbar` is swapped by the pump whenever the song or the settings
  change, so state held inside it folds the strip away mid-song. Outside, nothing in the fragment has
  to remember anything and there is nothing to re-sync after a swap — which a JavaScript toggle would
  have to do, since the element carrying `aria-expanded` is the one being replaced.
- **It must not survive a navigation.** Every tab is an ordinary link, so arriving at `/queue`
  renders the checkbox afresh and unchecked. A `prefs::` cookie — the mechanism the browse bar's ⋯
  toggle uses — would remember it for ever, which is the opposite of what a mis-tap guard wants.

`visually-hidden` rather than `display: none` on the checkbox, because it is a real control a
keyboard has to reach; the focus ring is drawn on the label through `#nowbar-controls:focus-visible ~
#nowbar .controls-toggle`.

`TransportBlock` lost its `target` and `query` fields with this. They existed because the Now tab's
card drew the same four buttons and wanted them back as `#player`; with one caller they are the two
constants in `_transport.html`.

## Getting to the other remote

*Open in browser* on the machine card is `<a href="{{ status.connection.address }}"
target="_blank" rel="noopener">` and nothing else. No route, no handler, no trait, no state: the
address is the one the card is already printing, so the link and the line above it are one field
rendered twice.

**There is no host seam behind it, and none is needed.** All three webviews route a URL that is not
their own loopback out to the real browser: wry's `with_new_window_req_handler`
(`km-remote/src/desktop.rs`), Android's `shouldOverrideUrlLoading`, iOS's `decidePolicyFor`. The
YouTube link in `_rows.html` exercises all three. A `POST /browser` route with a trait behind it
would carry an answer every host already agrees on.

**`target="_blank"` is the load-bearing part.** It is what makes wry see a *new window* request; a
same-window navigation would take the application's own window to the machine and strand somebody
there. The two phones accept either, but the attribute is what the desktop needs and what the
YouTube link already uses.

So the four seams the crate header names — data directory, bound address, shutdown, discovery — are
the four, and every one of them still differs between *hosts*. The one that differed between two
runs of one host was this, and it stopped being a question the moment the destination became the
machine rather than this process.

## Looking at the network is not switching machines

`Connect::rescan` answers a `Scanned` — `Nothing`, `Using`, `Already` or `Found` — rather than an
`Option<String>` naming the address it had *already moved to*. The shape is what carries the fault:
an `Option<String>` has nowhere to say "found one, kept the one I had", so the button that browses
also switches, and `Mdns::browse` takes the first service to resolve rather than the best one.

`Link::rescan` clears the pin first and unconditionally, then branches on
`MachineClient::connection().online`. Only the offline branch calls `point_at`. `Connect::use_found`
is the fourth action — `POST /machine/use` — and points without pinning, which is the distinction
`connect_to` exists on the other side of.

Two things about testing it are worth knowing. Reachability is the event stream's to set, so
`MachineClient::set_reachability` is `pub(crate)` rather than private: a link that is *answering* is
the state the whole branch turns on and nothing outside `client.rs` could otherwise produce one. And
the addresses are compared as strings, which is safe only because a browse produces
`http://<IPv4>:<port>` and nothing else — the TXT record is shape-checked to exactly that and the
sweep builds it itself — where an address somebody typed would need `find::normalize` first.

On the page, the offer is `MachineFound`, rendered into `#machine-found` in `setup.html` by an
out-of-band swap. It has to be a sibling of `#machine` rather than part of it: the pump republishes
the card whenever the connection changes, which is the same reason the address `<details>` is out
there. `views::with_toast_and_oob` is what answers with all three at once — the card htmx aimed at,
the offer that swaps itself, and the toast. Every `/machine/*` action passes the offer block, empty
unless a rescan filled it, so nothing can leave a stale recommendation on screen.

## The core, and why a `main` is not portable

`km-remote-core` holds the mirror, the favorites database, the machine client, discovery, sync and
the server. `km-remote` is the desktop's command line, data directory, banner and window.

**The rule the split enforces is one sentence: a `main` is the one part of a server that does not
travel.** It reads `argv`, derives paths, installs a log subscriber, prints, and blocks on Ctrl-C, and
a phone answers all five differently. The core's *absences* are the enforcement — no `clap`, no
`tracing-subscriber`, no `directories`, nothing that prints — three of which are a compile error
rather than a review comment.

**Four phases, not a callback.** `bind` → `open` → `warm_up` → `serve`, each an ordinary `async fn`. A
`ready` callback was the obvious alternative and is worse on three counts: it cannot be awaited, it
forces a `Send + 'static` closure across whatever boundary the host is on, and it inverts control for
nothing. **A phase boundary is somewhere a test can stand and a host can do its own work.** `Bound`
being a type of its own is what makes `bind: 0` answerable, which a phone needs because two copies of
an app cannot argue over a fixed port; it is also why `Bound::url()` is the loopback form and not the
bound address formatted, since `0.0.0.0:8179` is a fine thing to bind and not a thing anything can
connect to.

**Discovery is a trait and could not have been a `cfg`.** An mDNS browse on Android sees nothing
unless Java holds a multicast lock; on iOS it needs a prompt only the application can raise. The
decisive point is that the *same* Android build wants a real browse while it holds the lock and none
when it does not — **a difference between hosts and not between platforms**, which no
`#[cfg(target_os)]` can express even where it guesses the platform right.

**`Locator::look` answers a list of sightings, and `watch()` is what separates listening from
asking.** `Mdns` owns a `km_api::discover::watch::Watcher` — one long-lived daemon, opened lazily so
that building a `Config` in a test opens nothing — and its `look` is a snapshot that waits only when
the registry is still empty. `Sweep` and `NoLocator` answer `None` from `watch()`, so `Radar` polls
them instead. **The registry lives inside the locator that can listen rather than above the trait**,
because a registry above it would make push and poll look alike to a caller while being nothing
alike underneath: a sweep's `last_seen` would mean *the last time I walked a thousand addresses*, and
a subscriber woken by it would be woken by a poll it paid for itself. `Radar` is the one place that
decides between the two, so no consumer has to know which it has.

**The sweep's probe stops throwing its answer away.** It fetched and parsed the whole `/discover`
document to check the `app` field and returned `bool`; it returns a `Sighting` now, so the one
platform that may not multicast feeds the same identity-anchored decision mDNS does — and `Sweep`
can be told to *hunt* one id, which is what stops a house with two machines being a coin toss. It
still reports **the address that answered** rather than the machine's own `urls.first()`: an
announcement's A records are unverified and the machine's ranking is the better guess, but an
address this device just probed and got a reply from is proof, and the machine's preferred one may
be on a subnet this phone cannot reach.

**`machine_watch` selects rather than ticks.** It waits on the radar's change signal, on somebody
coming back to a page, and on a twenty-second fallback. The fallback stays for three reasons that
are easy to lose: `Sweep` has nothing to push, a watcher whose daemon would not open needs somebody
to call `poke` on it, and writing the record down is a timer's job. What changed is that a machine
reappearing at a new address now moves the connection in under a second rather than up to twenty.

**Android needs no Java change for any of this, which is the neatest part.** `onStart` retakes the
multicast lock *before* the WebView resumes; the resumed page reopens its event stream; the handler
for that already calls `Machine::wake`. So `wake` pokes the radar as well as the reconnection loop,
and the lock is guaranteed held by the time a query goes out — no upcall into Java and no seventh C
function. `MachineClient` carries a **second** `Notify` for it rather than sharing the existing one:
`notify_one` stores a permit and hands it to exactly one waiter, so two listeners would race and the
loser would be the event stream.

**Shutdown is an injected future**, and stopping uses `notify_one` rather than `notify_waiters` so the
permit is stored — a window closed during a long first import is a real case, and with
`notify_waiters` it would hang.

**The catalog refresh must not run before the server is called.** Ahead of it, on a cold first run
every request sits in the accept backlog for the length of a six-figure import. On a desktop that is
a slow tab; on a phone it is a launch a host cannot distinguish from a hang. `spawn_warm_up` puts it
behind the pages, which are served from whatever the mirror already holds — the offline app's whole
premise.

Six things that each decided a piece of the design:

- **The event stream is re-broadcast, not proxied.** The argument that first decided this was about
  permissions — a browser cannot put an `Authorization` header on a WebSocket, so a page following
  the machine directly would break the moment anybody put `events.subscribe` behind the password —
  and that particular risk is gone, since the stream is outside `/api/v1/admin/` and cannot be moved.
  **The other three reasons are what it now rests on**, and they were always the larger part: the
  *offline* remote has no machine to follow at all and needs the hub regardless; one hub means one set
  of renderers across both modes; and a page that opened its own socket to the machine would hold one
  of a browser's six connections per tab. Kept, with the reasoning corrected rather than the code.
- **The state event is split, and that is why nothing needs `hx-preserve`.** The player *card* is
  republished only when the song or the settings actually change; the elapsed time and progress bar are
  a separate fragment sent at most once a second. The reference implementation replaced the whole card
  on every push and had to protect each control from being rebuilt under a finger; **solving it at the
  source removes the problem instead of guarding against it.**
- **The latest value of every state-bearing event is replayed to a new subscriber**, and the queue
  *count* is a different event from the queue *list* — one event cannot be swapped into two places, and
  the badge is on every page while the list is on one.
- **A favorites folder is assembled in Rust, not in SQL.** It is a bounded personal list, so pushing
  an `id IN (...)` into the catalog would put the remote's furniture inside the machine's catalog
  for a query never large enough to need SQL's help.
- **Nothing answers with a non-2xx for an ordinary refusal.** htmx does not swap on an error response,
  so a queue-is-full reported as a 409 would leave the button visually dead and the reason nowhere on
  screen. Refusals become toasts, swapped out of band so a message lands on whatever page is open.
- **A command the machine never acknowledged is not a failure.** The badge reads `sent` and the toast
  is not red, because the song is almost certainly queued and what is missing is the confirmation.

**Two rules the connection card had to obey rather than discover.** A fragment carrying an input must
not be republished under somebody's finger, so the address box lives *outside* the swapped card — the
pump ticks once a second and would otherwise eat a half-typed address. And connecting deliberately
does **not** remember the address: *nothing is remembered until it answers* is what stops a dead
address short-circuiting the browse for ever, and the watcher writes a real machine down within twenty
seconds anyway. **The omission looks like a bug and carries a comment saying it is not.**

`pinned` is an `AtomicBool` read on each tick rather than a `bool` captured once: that was right while
only a command-line flag could pin a machine, and a person typing an address into the card pins one
too, so a loop holding the startup value would go on obeying an instruction that had since been
withdrawn.

## How the online mode is wired

`OnlineSongs` and `OnlineMachine` reach the catalog and controller this process already owns — no
HTTP, no serialization and back. **But every *operation* goes through `km_api::ops`, the same
functions the JSON endpoints call.** The half that would have diverged is not the operation, it is the
**publishing**: queueing a song is one call to the controller, and queueing a song *and telling every
open page about it* is two, of which the second is invisible when forgotten. That is why `ops` exists
as a module rather than as a handler body.

**There is no guard on this page, and its absence is the same claim a guard would make.** A guard
mapping every route the remote serves to the API route id it would exercise, and asking
`ApiState::authorize` about it, would establish that nothing the remote can do is anything the API
would not already allow the same caller. Every one of those routes is outside `/api/v1/admin/` and
always will be, so that is true by construction and there is nothing to refuse. The `/login` page
and the token cookie went with it: a login that can never be needed implies the person has forgotten
something.

**The cookie's reasoning moved one crate over rather than being lost.** A browser cannot be told to
put an `Authorization` header on a link, so the owner's page at `/admin/` — where every control *is*
an admin action — keeps the token in an `HttpOnly` cookie and rebuilds the header before the check.

Two faults found by running the guard while it existed, both still true of the page that has one.
**`ConnectInfo<SocketAddr>` rejects when the connect info is absent**, so a login handler taking it
directly 500s on any transport that has none. And **a refusal answered with a redirect *status*
leaves htmx doing nothing at all**: an htmx refusal is a 200 carrying `HX-Redirect`, a navigation is
an ordinary 303.

## Mirroring the catalog

`GET /api/v1/songs/export` — NDJSON, one song per line, keyset-paged on `?after=`, capped at ten times
the search cap because a search page is read by a person and an export page by a program. The
catalog carries a `catalog_version` bumped inside the same transaction as every install and
uninstall **that changed anything**, returned as an `ETag` and repeated in the always-public
`/discover`.

**That "changed anything" is the difference between the counter working and not.** Startup reinstalls
every configured package at every start, so an install that bumped unconditionally moved the number on
every start and told every mirror to re-download an identical catalog — measured as versions 3, 4, 5
across three restarts of a one-package catalog. `install` now takes a package digest before and
after itself and bumps only on a difference; the digest selects through the same column list the
export sends, so the two cannot drift about what "changed" means.

**The library's own version test could not see this**: it installs two *different* packages, where
both installs legitimately move the counter, so only reinstalling one package distinguishes a working
guard from none.

**Tags are mirrored where `lyric_preview` deliberately is not**, and the test between the two is what
the offline pages actually draw. No page here shows a preview, so carrying it would mean a column, an
INSERT, two SELECT lists, a `read_song` and a place in `discard_unless_current` for a field nothing
reads. The tag
filter is a control on a page that has to work with the machine switched off, which is what this
database is for — so `songs.tags` and a `song_tags` index table are here, filled by `Mirror::replace`
for `Library::install`'s reason, and `Mirror::conditions` emits the same single `EXISTS` over an `IN`
list that `km_catalog::SearchQuery` does. The two sides narrow identically by construction rather than
by coincidence.

**A mirror without `songs.tags` is discarded rather than given the column.** A tag is not derivable
from anything in this file: it comes off the wire.
Adding the column would leave every mirrored song reading as untagged, `song_tags` an index over
nothing, and the picker drawing nothing — and `sync::refresh` would never repair it, because it
downloads only when the machine's catalog version differs and this phone's has not moved. Discarding
costs one re-download of what the machine still has, and `favorites.sqlite` is a separate database
that goes nowhere. The `meta` row goes with the songs for the reason `discard_unless_current` gives:
leaving it would answer "already up to date" over an empty table.

**`songs.content_hash` took the same branch, for the same reason and with a sharper failure behind
it.** It comes off `SongDto`, which carries it since favorites began rejoining on it, and nothing
here can derive one. A column added empty would read as *"this package never recorded a hash"* — a
state `Mirror::resolve` cannot tell from the truth, because a null there deliberately means *unknown*
rather than *different* — so every favorite would fall silently to the number rung and the repair
would never fire. Discarding is one re-download; adding is a feature that looks installed and is not.
Two indexes come with it, `songs_content_hash` and `songs_package`, so resolving a folder is a seek
per song rather than a scan per song.

**Package names are mirrored in a `packages(id, name)` table, and a mirror without one is
discarded.** A song row carries only its package id, so `sync::refresh` reads the public
`GET /api/v1/packages` after the export and `Mirror::replace` writes both in one transaction. A failed
read keeps the songs and stores no names, and `Mirror::packages` then lists a package under its id.
An old mirror is discarded rather than given an empty table, for the reason tags are: names come off
the wire, and an empty table beside a current mirror would stay empty until the catalog version moved.

**`km-catalog` answers the same question for the machine's own catalog and does not share a line
with this.** The two schemas are in two crates with nothing but a grep connecting them, and changing
one without the other makes the remote answer every browse with `no such column`.

## A folder as a code, and the collection as a file

Two formats, one write. `km_remote_pages::share` encodes one folder as decimal digits;
`km_remote_pages::backup` carries the whole collection as JSON; both end at
`Favorites::add_songs`, which is `INSERT OR IGNORE` in one transaction and cannot remove anything.
The *why* of each is a decision entry — this is what the code does and what was measured doing it.

**Only the file carries an identity.** `SongDoc` gained optional `package_id` and `content_hash` at
format 2, and unlike the `title` and `artist` beside them they are *read* on the way back — which is
what lets a collection restore onto a machine that banks the same package differently. The code
format is untouched at `2`: it is all-decimal to sit in QR's numeric mode and is pinned byte-for-byte
to the sibling project by `a_code_this_build_writes_is_the_code_the_sibling_project_writes`, and
carrying a package id through it would need a dictionary and a format digit that sibling phones would
refuse.

## A favorite that knows what it is

`favorite` carries `package_id` and `content_hash` beside `song_code`, both nullable, and
`Songs::resolve` walks the three rungs the decision sets out: the pair, the hash alone, the number.
`Favorites::reconcile` is the single write that both fills those columns in and moves a row whose
song now sits under a different number — across every folder holding it, since a song is commonly in
several.

The number rung is filtered by `SongRef::contradicted_by`, which drops a song whose own hash is a
different one from the favorite's and keeps every song that records none. Three implementations walk
the rungs — `Mirror::resolve`, the machine's own `Songs` over `km-catalog`, and the stub in
`tests/pages.rs` — so the rule lives on `SongRef` where all three reach it rather than three times
over.

**What cannot be placed is reported, not only dropped.** `songs_for` answers a `Listing`: the page,
and the `(SongRef, Miss)` pairs the catalog refused. `not_here_lines` turns them into a count
sentence plus the restore report's own `unplaced_lines`, and they ride to the markup on
`ListBlock::not_here` — `#list`, which a search replaces whole, rather than `RowsBlock`, which *Load
more* appends. They are collected only for an unfiltered folder, since a resolve failure is not
something a filter narrowed.

**The repair fires where somebody looks.** Drawing a folder is the frequent operation that runs
against a fresh mirror, so that is where `browse` calls `resolve`, collects
`Reconciliation::of` for every row that changed, and writes them back. `Reconciliation::of` returns
`None` when nothing moved and the identity already matches, which is what keeps a folder opened twice
in a row from writing every row back a second time — and `reconcile` returns early on an empty slice
rather than taking a write lock to say so.

**Added, never discarded**, where the mirror beside it does the opposite. A machine still holds what
the mirror copies and nothing holds a copy of this, so the columns arrive by `ALTER TABLE` and rest
empty. That is safe precisely because a null resolves exactly as every favorite did before the
columns existed.

**Both live in `km-remote-pages` rather than in `km-remote-core`, and the layering settles it rather
than taste**: the core implements the traits this crate defines, so it depends on this one and never
the reverse. The handlers are the only caller either has.

**They are entered from different places, which is the one thing about them that is not common.**
Sharing is offered by a folder's own header in `_browse.html`; the backup is a `.big-choice` on
`setup.html`, gated on `Capabilities::favorites`. `BrowseBlock::shows_backup` was the gate while the
link was in the browse bar and is gone — the capability alone answers it now, because the Setup tab
has no mode and no folder to narrow it. The three backup handlers pass `"setup"` as the chrome's tab
and `backup_choose`'s step-back is `/setup`; the share handlers still pass `"browse"`.

**The codec's interop test is verified against the other program's encoder, not against its
documentation.** `a_code_this_build_writes_is_the_code_the_sibling_project_writes` pins
`2400020040821110991071001200562` for a folder called `Rock` holding 1001 and 2005, and that string
was produced by running `favsync.Encode` in the Go project rather than derived from the format table
twice. Deriving it twice is exactly how two implementations of one wire format come to agree with
each other's documentation and not with each other.

**The QR is an SVG, and that deletes a calculation rather than adding one.** The reference
implementation spends a long comment choosing a pixel scale — target roughly 900 px, because the
page draws the code at about 350 CSS pixels and a phone is three device pixels to one, so a code
generated at its natural size reads as a blur when scaled up tenfold and one generated far larger
loses whole modules to a downscale. A vector has no scale to choose: `module_dimensions(1, 1)` makes
the `viewBox` the module count and the browser rasterizes at whatever the screen is, with the
library's own `shape-rendering="crispEdges"` keeping the modules square. A 25-module code comes out
33 units square, which is the quiet zone the standard requires being drawn rather than trimmed.

`qrcode` takes `default-features = false, features = ["svg"]` here. The defaults are two renderers,
and the `image` one would pull that crate into a library compiled into the appliance's binary, an
APK and an iOS app for a page none of them draws. The `svg` feature is string formatting with no
dependency at all. The workspace entry had to lose its defaults for a member to be able to say that
— cargo refuses `default-features = false` on a member whose workspace entry does not already
declare it — which turned out to be an improvement: `km-display` and the machine use `QrCode::new`
and `to_colors` and want neither renderer.

**The end-to-end check is worth describing because it is the one nobody runs by accident.** Encode a
folder, fetch `code.svg`, parse its one-rect-per-module path back into a bitmap, and hand that to
jsQR — the same decoder the receive page loads. The string that comes out has to be the string that
went in, byte for byte. Done from a scratch script rather than a test, because rasterizing an SVG in
Rust would mean a renderer in the dev-dependencies to check a property that cannot drift silently:
if this breaks, no folder can be shared at all.

**`kind` is what makes the backup document refusable.** XML gives the sibling project identity for
free — a root element is what turns away a photograph handed to a file picker — and JSON has no
equivalent, so a required `kind` field does that job and a `folders` array alone is refused. The
`format` number runs the other way and is reported rather than enforced, which is
`km-package-builder`'s rule and its reasoning. `SongDoc::code` is a `String` for the same family of
reason as that crate's `i64`s: serde cannot refuse one row, so a narrow type would let one mistyped
digit fail a whole document.

**The dangerous case is an empty mirror, and it has a test in both features.** `known_songs` drops
codes this device's catalog cannot show, which keeps a folder's count honest against its listing —
but a phone that has never reached a machine has *no* catalog, which is this app's normal starting
state. Filtering against it would discard an entire collection at the moment somebody was restoring
it and report the loss as a count. So a catalog holding nothing keeps everything, and the resulting
drift is the state a favorite outliving its package already produces.

**The restore route takes a text field rather than multipart.** `pick.js` reads the chosen file with
a `FileReader` and posts its text as `document`, which is the same field the paste box uses — so
there is one handler on a `String` body, which is what `form.rs` exists for, and no `multipart`
feature added to a table shared with the machine and the owner's page. The paste box is also the only
path that works with no JavaScript at all. Its route carries an explicit
`DefaultBodyLimit::max(RESTORE_LIMIT)` on the method router: the default is 2 MB, applies to a
`String` too, and answers `413` — which htmx will not swap, so the Restore button would go dead with
the reason nowhere. `MAX_DOCUMENT` inside the handler is the refusal that gets a sentence.

**`scan.js` carries no prose, and that is a correctness property rather than tidiness.** A static
file cannot go through the `|t` filter, the catalog scanner reads templates only, and
`a_portuguese_page_has_no_english_and_no_untranslated_keys` fetches pages and never sees the script —
so an English string in there would show on a Portuguese page with nothing to catch it. Its five
sentences come off `data-` attributes on `#scan`, and
`the_scanner_takes_its_sentences_from_the_page_rather_than_the_script` greps the file to keep it that
way.

**jsQR rather than `BarcodeDetector`**, which is in Chrome and so in Android's WebView and not in
WebKit — relying on it would leave the iPad without the half that matters most, since iOS's camera
app handles a plain-text QR poorly and that is the reason to scan in-app. It is a quarter of a
megabyte, it is Apache-2.0 so the license travels with it, and it is compiled into the machine's
binary as well because this crate is linked there and `include_str!` is unconditional. `Bundling
assets` admits an exception for size on the order of the 31 MiB SoundFont or for a license that
forbids redistribution; a page the online mode never draws is neither, so the bytes are paid. Only
`share_receive.html` loads it.

**The camera needs a potentially-trustworthy origin, so `--lan` has no scanner.** All four shells
load `http://127.0.0.1:<port>/`, which qualifies; a phone pointed at a `--lan` remote's LAN address
over plain HTTP does not, and `navigator.mediaDevices` is simply undefined there. `scan.js`'s opening
feature test hides its button and leaves the paste box, which is the reference implementation's
"the camera is never required" design handling a case that project never met.

## The shells grew three callbacks

All four shells needed the same two grants, and the page could not have made either.

**The camera and the file input fail in the same shape, and it is the shape worth knowing.** An
unanswered `PermissionRequest` on Android — or an unanswered `decisionHandler` on iOS — leaves
`getUserMedia` pending for ever rather than failing, so the receive page sits on "Starting the
camera…" with no error and no log line. An unanswered `ValueCallback` leaves `<input type="file">`
*permanently* dead, which only shows up on the second attempt at restoring. Every branch of every one
answers, including Android's `results.length == 0` — how it reports its prompt being dismissed by a
tap outside rather than answered either way, and the branch a reading of the happy path misses.

**Android's file-chooser intent is built by hand as `*/*`, which is the opposite of what it looks
like.** Calling `params.createIntent()` would turn the page's `accept` list into a MIME filter, and
Drive and Dropbox report their own types for what they hold — so a backup plainly visible in the
picker becomes impossible to select. The `accept` list is for iOS, which maps it to UTTypes.

**The copy loop is written out because `InputStream.transferTo` is API 33** against this
application's `minSdk 26`, with no core-library desugaring (it needs a dependency this project has
none of by design) and `lint { abortOnError = false }` so `NewApi` would not stop it. The failure
would be a `NoSuchMethodError` on exactly the old hardware `armeabi-v7a` exists for — and on no
device in either port's test record, which is why it is written down here.

**`CAMERA` implies two features, not one.** Android makes `android.hardware.camera` *and*
`android.hardware.camera.autofocus` required, either of which would filter the app off a device with
no rear camera. Both are un-required beside `android.hardware.wifi`, and `aapt2 dump badging` on the
built APK is the check — all three read `uses-feature-not-required`.

**iOS needs four things and none is redundant with the others**: the plist key, without which the app
is *killed* rather than refused; `allowsInlineMediaPlayback`, or the preview goes fullscreen over its
own page; `mediaTypesRequiringUserActionForPlayback = []`, or `play()` is blocked because it runs
after `getUserMedia` resolves and the tap no longer counts; and
`requestMediaCapturePermissionFor`, which replaces WebKit's own prompt — never remembered in a
`WKWebView`, so it would otherwise be raised on every scan. That last one is what makes the receive
page's auto-start viable rather than a prompt per visit, and granting there removes the gesture
requirement with it.

**The 102 guard was already there, and what it gained is a third reason.** `isHarmless` filters
`WebKitErrorDomain` 102 and `NSURLErrorCanceled` in both failure callbacks. Turning a navigation into
a download *cancels that navigation*, so a saved backup now arrives as a 102 routinely where it used
to be the rare consequence of a refused foreign link. Narrowing that guard to "only cancellations on
a foreign host" would put a failure screen up every time somebody saved their favorites — and the Go
remote hit worse, because its failure path restarted the server, so saving a backup tore down the
server that had just served it. Neither shell here restarts anything on a load failure, so the cost
is a page taken off screen rather than a server; the guard is defended in a comment for that reason.

**A download goes to `temporaryDirectory` on iOS and to the picker's own stream on Android**, and
neither app ever holds the file. `Documents` was refused because it is iCloud-backed and would make
`excludeMirrorFromBackup`'s literal array a three-file array; `ACTION_CREATE_DOCUMENT` was chosen
because it lists on-device storage, Drive and Dropbox and needs no storage permission and no
`FileProvider`. Do not turn the export into a `blob:` URL on Android: `staysInside` returns true for
any non-http scheme, so it would stay in the WebView, which cannot render one, and nothing would
happen at all.

**The desktop window makes the same two grants through `wry`'s seams**, which exist: a permission
handler that allows `Camera` and denies everything else — `Microphone` explicitly, since mic audio is
mixed in hardware — and download handlers that log where the file went. Downloads already worked
before those, because wry's default allows every one "to match browser behavior"; what was missing
was any statement of where it landed. Linux needs neither, because it opens the real browser.


## The remote on Android

**`state.rs` had no Android in it, and that was the load-bearing decision** — everything genuinely
easy to get wrong lives there (an idempotent start, a stop that must not block its caller, and a
superseded run that must not overwrite the state of the run that replaced it), exercised by an
ordinary `cargo test` on the machine doing the building. It is now `km-remote-host`, shared with iOS.

Three things about the FFI that are not obvious:

- **The port is published once `serve` has begun, not once `bind` has answered.** The port is knowable
  after phase one, but nothing answers on it until phase four, and a WebView pointed there in between
  gets a refusal it cannot explain. So the port is written **last**, because the host polls it and
  reads the rest once it is non-zero.
- **The generation counter is imported wholesale**, with its comment: without
  it, *"the guard against double-starting becomes a guard against ever starting again"*. Rust needs it
  more sharply, because **dropping a `tokio::runtime::Runtime` blocks** until its blocking tasks
  finish — and what is on a blocking thread here is a network browse or a request to a machine that
  has been switched off. So `stop()` takes the runtime out under the lock, clears the port on the
  calling thread, and hands the runtime to a thread of its own. It returns at once, which is what
  makes it safe to call from `onDestroy` on the main looper.
- **Nothing is thrown into Java.** A panic cannot unwind into the JVM; what happens next is a choice,
  and the choice is to log and return a fallback. An exception reaching an Activity that polls every
  hundred milliseconds is a crash, and a remote that cannot find its machine has better things to do
  than take the app down.

**What was not copied from the reference implementation, and why each absence is a design.** That
project is the same shape — a WebView on a loopback server inside the app — and most of what goes
wrong there cannot happen here:

| Its problem | Why it is not ours |
|---|---|
| The server is `exec`ed as a child process, named `lib*.so` so the APK extracts it | A **Go** constraint: a cgo-free Go binary cannot be loaded as a library. This is a real `cdylib`, so nothing keeps two copies on the device |
| The port is scraped from the child's stdout | Read across the FFI |
| A 20 s startup deadline reported a healthy server as dead, because a first run downloads the catalog before it listens | The import runs **behind** the pages. The long-deadline discipline is kept anyway; the failure it was written about cannot occur |
| A foreground service, and the permission trap and crash loop that come with it | Nothing to hold: no session slot on the machine, and an in-process server whose port does not move across a background |
| Download, file-chooser and permission callbacks | **Ours too now**, and the two unanswered-callback traps with them. See `The shells grew three callbacks` below — what differs is that its `onShowFileChooser` builds its own `*/*` intent, its copy loop is written out because `transferTo` is API 33, and it un-requires *two* implied camera features rather than one |
| Two environment variables carrying a time zone, because Go hardcodes `time.Local` to UTC | Rust has no such thing. The only wall clock is SQLite's `datetime('now')`, UTC by definition and read only to sort by |
| A shim to rebuild the event stream after a suspension | **Half right, and the wrong half cost an evening.** A plain `EventSource` does reconnect on its own and the hub does replay, so none of the extension's connection handling is reproduced here — but a WebView background fires no event either of those hangs off, and the page came back holding a stale banner. There is a shim now: three lines waking the stream on `visibilitychange`, which is not the same thing as re-registering swap targets |

**One screen exists here that the reference does not have**, and the reason is a property of this core.
There, not finding the unit is a failure. Here **not finding a machine is not a failure** — browsing,
searching and favorites all answer from the mirror — so on a network with no mDNS the app starts
perfectly and a failure screen would never appear, which is exactly when an address field is needed.

## The remote on iOS

The third host, and the one that proved the crate boundary was in the right place: the pages crate was
not touched at all, and the core gained one additive, default-off feature.

**The `Locator` is an argument to `start`.** The two shells cannot share a default — Android may
browse while Java holds the multicast lock, iOS may not browse at all. It is also what keeps
`cargo test` from asking a Windows developer's firewall on every relink, without a `#[cfg(test)]`
inside `serve` substituting a null locator: **a test is a host, and a host chooses its locator.**

### Three things the C surface needs that JNI did for free

- **`catch_unwind` on every export, by hand.** An `extern "C"` function is `-unwind` in this edition,
  so a panic crossing one aborts the process — which somebody sees as the app vanishing while a timer
  polls it ten times a second. **This makes the panic hook load-bearing rather than a nicety**: with
  every panic caught and swallowed, a panic with no hook would be completely silent, and the only
  symptom would be a starting screen that never leaves.
- **The inbound strings are copied before anything is spawned.** Swift's `withCString` frees its buffer
  the moment the closure returns. This is the worst bug the reference implementation ever had and the
  shape of it is the lesson: the freed bytes read back as a *plausible* address, so discovery was
  skipped and a dial ran to a timeout — presenting as a slow first run rather than a crash — and it was
  invisible in the simulator, where the freed bytes happened to begin with a NUL.
- **The outbound strings belong to the library.** Swift polls at ten hertz, so a fresh allocation per
  call leaks on every one. Each string function keeps its answer in a static and hands out a pointer
  into it, replaced only when the value changes.

**The header is hand-written**, with a test keeping it honest. cbindgen was declined — another tool, a
config file, a pinned version and a step, for a surface whose whole design principle is that it never
grows. The test reads both files as *text*, which is what lets it run on the machine doing the
building: the FFI module is behind a target `cfg` and is not compiled there at all, so a test that
needed it compiled would be a test that never ran.

### The sweep, and what it cost to avoid Bonjour

- **The feature gates less than it looks like.** The subnet walk and its refusals are `std`-only
  arithmetic, compiled and tested always; only the interface enumeration and the HTTP probe are behind
  the flag. Gating the policy would have kept it out of the ordinary test command and dragged the
  feature list, three alias strings and an assertion into the change — for a decision that is not about
  a dependency at all.
- **An answer is a discovery document naming this application, not an open port.** The cost of
  believing a router's admin page is not a retry but a *memory*: an address is written down as soon as
  it reports online and preferred over looking again, so a false positive is a remote that opens on a
  printer every morning.
- **The shared browse timeout is advisory here, and only here.** It is sized for mDNS; a sweep held to
  it walks a tenth of a /22 and calls the result "nothing found". So the sweep carries its own budget
  and takes the larger — rather than raising a shared constant and slowing every desktop start.

The concurrency is `JoinSet` plus a fair `Semaphore` and not `buffer_unordered`, which compiles in this
tree only because reqwest and axum unify a feature in — exactly the accidental-unification hazard the
`km-wallpaper-pack` exclusion was written about. Bridging blocking to async uses `try_current` rather than
`current`, so a sweep built with no runtime answers "nothing found" instead of panicking.

### Two things found by building it

**The virtual-adapter list did not know what a `bridge100` was.** macOS names its VM bridges that, and
they are up with a valid private address, so the sweep ranked one ahead of the Wi-Fi card. That list is
the *machine's* code, so this is the remote's second reading of it improving the address the machine
puts on screen. `br0` is deliberately not matched: on a hypervisor host that is very often the real
LAN.

**...and `Mdns` was reading none of it.** The sweep locator ranks; the mDNS one took the first IPv4
out of `get_addresses()`, which is a `HashSet` — so against a Windows machine carrying WSL and
Hyper-V an Android remote latched onto `172.17.0.1` or `172.28.0.1` about as readily as the real LAN
address, and picked a different one on each run. It reads `km_api::discover::resolved_url` now, which
is the same function `browse` uses: the machine's own `url` TXT record, falling back to the
best-ranked address. That record is also what keeps `Mdns` free to return at the **first** machine
that resolves, which is its documented job — an announcement carries only the addresses on the
interface it left by, but it carries the whole TXT set every time. See `The advert names the address
the machine chose` in `docs/decisions/api-and-network.md`.

**The link line is `-liconv -lSystem -lc -lm` and nothing else.** An API crate depending on the
audio one would want `AudioToolbox`, `CoreAudio` and `AVFAudio`; the `km-queue` split takes that
away, so **a remote that never plays a note carries no synthesizer**. It is asked of
`rustc --print native-static-libs` rather than maintained by hand.

### Two bugs a device found in ten minutes

**The "no machine found" screen must read a live accessor.** If `machine()` returns what phase 2
found and is never written again, the watcher re-points its client every time it finds one afterwards
and none of that reaches the value both shells poll: each shows the screen, polls a constant, and
waits for ever while its own log shows the server finding a machine twenty seconds later.

**Android hides that and iOS does not.** The multicast lock is taken early there, so the first browse
usually succeeds and the screen is rare; on iOS the first sweep loses its packets to the
local-network prompt, so the screen is the *ordinary* first-run case. One accessor, **one change,
both shells** — which is what extracting the host crate buys.

**A superseded page load is not a dead server.** When the watcher finds the machine and swaps in the
web view, the load it replaced reports a cancellation — and reading every navigation error as the
server having died puts the failure screen on screen at the exact moment the machine was found,
offering a *Try again* that "fixes" it by restarting something that was never broken.

**Safe areas needed nothing**, which is the one place iOS is simpler than Android: the web view
reports the insets to the page, the stylesheet already declares its bottom inset with an `env()`
default, and the whole handling is pinning three edges to the safe area and the bottom to the view —
because the tab bar is meant to reach the edge, and pinning the bottom too floats it above a blank
strip.

## What language a page is in

`Prefs::read` resolves it off the request — the `km_locale` cookie first, `Accept-Language` second,
English if neither names a language this build has. It is a preference like the other four in
`prefs.rs`, and negotiating it there rather than in middleware is what keeps it out of the five hosts
that mount this router. The `Set-Cookie` value is `km_locale::set_cookie`'s rather than `prefs::set`'s,
alone among the five, because `km-admin` writes this same cookie on an origin of its own.

**No template struct carries a locale.** askama 0.16's `render_with_values` propagates a values store
into every nested `{{ child|safe }}`, so `views::page`, `with_toast`, `toast_only` and
`with_toast_and_oob` put it in once and `{{ "tab-songs"|t }}` finds it anywhere below. The filter is
`km_locale::filters::t`, resolved by askama against the `filters` module `views.rs` imports — one line
wires 24 templates.

**Markup carries a key and nothing else.** A message that interpolates a title, a folder or a count is
composed in Rust through `Catalog::msg_with` and arrives as a field, which is the rule `model.rs`
already states about formatting: arithmetic in markup is where it stops being testable. `ListBlock`'s
count, the empty-list table, every toast and badge, the per-row song counts and the machine card's
copy count are all composed that way; `Mode::label_key`, `Mode::placeholder_key`,
`PlayerView::title_key` and `MachineBlock::how_key` return a key for the markup to render, which is
the same rule where the choice is a `match` rather than an interpolation.

### Rendering a refusal

`RemoteError::Unavailable` carries the machine's stable code and its prose. `views::message_for`
renders from `words::refusal_key(code)` out of this crate's own catalog; the prose goes to the log.
The three messages about a song select on `words::refusal_kind(code)`, which reads the kind off the
code's suffix — see `A refusal travels as a code` in `docs/decisions/api-and-network.md` for why the
kind has to reach the page at all.

A code this build does not know renders `error-unavailable` and logs a line naming it. `client.rs`
therefore forwards **every** 409's code rather than only the ones it recognizes: matching known ones
there would drop a newer machine's refusal to `Failed`, which is a red toast for something that is not
a fault.

### A pushed fragment carries the language it is written in

Everything above is about a *response*. The fan-out is not one, and for a while none of it applied:
`handlers::pump` is a single task per process, it has no viewer to ask, and it called
`Template::render` — which takes no values store. Every fragment it published came out as `⟦key⟧`, so
a page was correct as it loaded and overwritten a second later by the replay. The Now tab and the
machine card were the visible half; the queue list, the dot and the banner were the rest.

The locale is on the frame, because one rendered string cannot serve two phones in two languages:

| | |
|---|---|
| `sse::Frame` | gains `locale`, beside `event` and `html` |
| `Hub.latest` | keyed by `(Locale, &'static str)`, so the replay a page gets is its own |
| `Hub::stream(locale)` / `frames(locale)` | filter the live feed and the catch-up |
| `handlers::events` | takes a `HeaderMap` and reads the same cookie every other route reads |
| `handlers::everywhere` | renders one fragment once per `Locale::ALL`, through `views::render` |

**`CHANNEL_CAPACITY` scales with the number of languages** — `64 * Locale::ALL.len()`. The filter is
applied *after* the broadcast queue, in `frames`'s `Ok(frame) if frame.locale != locale` arm, and
tokio counts lag in every frame sent including the ones a subscriber will discard. A fixed 64 would
have halved each page's real headroom the moment a second language existed. The lag *recovery* needs
nothing: it reaches for `hub.snapshot(locale)`, which is per-locale already.

**The pump's dedupe still compares rendered markup**, which was and is the honest test of "would a
page look different?" — asked now of every language at once. `last_card` holds the whole
`Vec<(Locale, String)>`. The locales come from one `PlayerView` against constant catalogs, so they
change together; comparing the set rather than one of them keeps that an observation rather than an
assumption.

**`everywhere` takes a builder, not a template**, because a fragment may have to be *built* per
language and not only rendered per language. The machine card is the one that is: `394 songs in this
copy` is a plural over a count, so it is composed through `msg_with` and arrives as a field, and a
field can only hold one language. Every other fragment ignores the argument and is `|_|`.

Two tests hold it. `no_fragment_the_fan_out_pushes_is_ever_missing_its_words` renders all eight in
both languages and asserts `⟦` appears in none — verified by putting the bare render back and
watching it fail. `nothing_outside_views_renders_a_template_without_a_locale` scans this crate's
sources for `Template::render`, so a ninth fragment cannot be added the way the eight were broken.

### `km-remote-core` sends a code, and this crate writes the sentence

**In-process is not translated.** `km-remote-core` composing `RemoteError::Offline` in this process
does not put it in the viewer's language: that crate has no catalog and no viewer to ask, so
`The karaoke machine is not answering.` would land inside otherwise Portuguese pages, and so would
three more kinds of sentence. All four travel as facts instead:

| Carries | What |
|---|---|
| `RemoteError::Offline(&'static str)` | one of `machine::codes` |
| `RemoteError::Refused(&'static str)` | this device's own refusals — an empty folder name, a duplicate one, an empty address box |
| `Connection::reason: Option<&'static str>` | the same codes |
| `Connect::connect_to` / `use_found` / `refresh` | `Ok(Copied)`: an address, and `AlreadyCurrent(n)` / `Imported(n)` / `NotAnswering` |

`words::code_key` maps a code to a message id, beside `refusal_key` and `how_key`, and
`handlers::copied_sentence` composes the one sentence that has a count in it. A code this build has
no message for falls back to the generic failure in a toast and draws **nothing** in the banner: the
line above it already says the machine is not reachable, and a wrong reason is worse than none.
`codes::ALL` and `every_code_this_device_raises_reaches_a_message_that_exists` are what stop a code
being added without a message — a hole the compiler cannot see.

**`RemoteError::Rejected` still carries prose and is deliberately never rendered.** It is a 400 from
the machine, aimed at whatever built the request rather than at the person holding the phone, so it
goes to the log and the page says the generic failure — the treatment `Unavailable`'s message
already gets.
