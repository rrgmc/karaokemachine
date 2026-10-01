# Vendored assets

**Compiled into whichever binary links this crate** by `include_str!` in `src/lib.rs`, which serves
them at `/static/`. Nothing ships them as files and nothing stages this folder. See the *Bundling
assets* entry in `docs/decisions/`. Editing a file here changes the next build, and there is nothing
to copy anywhere afterwards.

That rule matters more here than it does for the curation tool. This crate ends up inside the
appliance's `.deb`, inside a macOS bundle, inside a portable folder and inside `km-remote`. A
stylesheet that failed to travel would leave a page that renders, ignores every tap, and reports no
error to anybody holding the phone.

Nothing here is fetched from the network at run time either. The machine may be on a home network
with no internet at all, and an unreachable CDN is the same inert page by another route.

| File | What it is | License |
|---|---|---|
| `htmx.min.js` | [htmx](https://htmx.org) **2.0.4**, unmodified | Zero-Clause BSD, verbatim in `htmx-LICENSE.txt` |
| `jsqr.js` | [jsQR](https://github.com/cozmo/jsQR) **1.4.0**, unmodified | Apache-2.0, verbatim in `jsqr-LICENSE.txt` |
| `app.css` | The remote's own stylesheet | Same as the repository |
| `live.js` | The remote's own event-stream listener | Same as the repository |
| `scan.js` | Reads a folder's share code with the camera | Same as the repository |
| `pick.js` | Feeds a chosen backup file into an ordinary form field | Same as the repository |

**jsQR is Apache-2.0, not MIT**, and the license text travels because that license requires it. htmx
gets the same treatment, and it is why both `-LICENSE.txt` files are routed at `/static/` rather than
merely sitting here. The copy in the Go remote this crate follows carries no license header, and its
own asset list records none. The license here therefore comes from upstream, not from that copy.

**A quarter of a megabyte, and it lands in the machine's binary too.** `karaokemachine` links this
crate as well as `km-remote`, and `include_str!` is unconditional. The appliance therefore carries a
decoder for a page its own remote never draws.

That is the accepted cost. A cargo feature would make this the first *compile-time* mode split here.
This crate's header says the templates branch on capabilities and never on a mode. The
`Bundling assets` decision admits an exception only for size on the order of the 31 MiB SoundFont, or
for a license that forbids redistribution. Neither applies. `share_receive.html` loads it alone
rather than from the shared head, so no other page parses it.

The htmx here is deliberately the same version `tools/cmd/km-package-builder` vendors. Two copies of
one library at two versions is a difference nobody would look for, when a swap behaves oddly in one
tool only. Update both together.

```sh
curl -sSL -o htmx.min.js https://unpkg.com/htmx.org@<version>/dist/htmx.min.js
curl -sSL -o htmx-LICENSE.txt https://raw.githubusercontent.com/bigskysoftware/htmx/v<version>/LICENSE

curl -sSL -o jsqr.js https://unpkg.com/jsqr@<version>/dist/jsQR.js
curl -sSL -o jsqr-LICENSE.txt https://raw.githubusercontent.com/cozmo/jsQR/master/LICENSE
```

jsQR's UMD bundle assigns `window.jsQR`, which is the name `scan.js` tests for and calls. Checking
that after an update is worth a moment. The wrapper prefers a CommonJS `exports` where it finds one.
Loading the file under Node to confirm it "works" can therefore take a branch a browser never will.

## Why htmx's SSE extension is not vendored

htmx ships server-sent events as a separate `ext/sse.js`, and `live.js` — about thirty lines around
the browser's own `EventSource` — does the job instead. Three reasons, and the third is the one that
decided it.

* `EventSource` already reconnects by itself. The extension layers its own connection handling on
  top. In the Go remote this crate follows, getting that to re-register its swap targets after an
  iOS suspend needed a shim of its own. Not reproducing the extension means not reproducing the shim.
* A second vendored file has a second version to keep in step with this one.
* The extension's main draw is `hx-preserve`, which keeps a control alive under a finger when a
  fragment is replaced underneath it. **The remote does not need it, because nothing replaces a
  control four times a second.** The machine's `state` event arrives every 250 ms, but the pump
  splits what it carries. The player *card* — every button, stepper and slider — is republished only
  when the song or the settings actually change. The part that moves constantly is a separate
  `position` fragment holding the elapsed time and the bar, at most one a second. Solving it at the
  source is what leaves a control never rebuilt under a finger, rather than rebuilt and put back.

**The one thing not vendoring the SSE extension costs.** That extension calls `htmx.process()` on
everything it swaps in, and `live.js` has to do that itself. htmx binds `hx-*` triggers only in
`htmx.process()`: once over `document.body` at `DOMContentLoaded`, and again over whatever htmx swaps
itself. The vendored 2.0.4 carries no `MutationObserver`, so a node some other script inserts is live
DOM that htmx has never seen. It renders correctly, and every control in it is inert.

That is not a theoretical hazard. `_player.html` is thirteen `hx-post` attributes inside the element
wearing `data-sse="player"`, and `Hub::stream` replays the latest `player` frame to every stream as
it opens. An identical dead card therefore replaces the server-rendered one htmx *had* processed,
within milliseconds of the page loading. The Now tab's buttons never work for anybody, and
`_queue.html` had the same defect for the same reason.

Nothing caught it, because `tests/pages.rs` drives the router as a service and asserts on HTML
strings. There is no browser in the loop, so nothing exercises htmx binding.

**So the rule for `live.js` is: everything it inserts gets processed.** `swap` builds the fragment in
a `<template>` and `replaceWith`s it, precisely so a node is left to hand to `htmx.process`. An
`outerHTML` assignment parses the nodes into the document and leaves no reference to them.

## The stream is closed when the page leaves the screen, and reopened when it comes back

This is the third thing about a browser that had to be learned here rather than guessed, and the most
expensive of the three. It presented as *the machine* being slow.

`live.js` opened one `EventSource` and left it open. That reads as obviously right, because the page
wants the stream for as long as it exists. It is wrong for a reason that is nowhere in this file. The
three tabs in `layout.html` are ordinary links, so a tab press replaces the page rather than keeping
it. The replaced page is not destroyed either. The browser keeps it in its back/forward cache,
holding its connection, where nobody can see it and nothing will ever close it.

A browser allows about **six connections to one host**. A navigation wanted six of its own: the HTML,
the four files in this folder, and a new stream. Two or three abandoned streams therefore left the
next request nothing to ask with. It queued until the browser evicted a cached page, which is tens of
seconds. `hx-sync="body:queue last"` on the body allows the document one in-flight request, so the
first thing to queue stopped every later tap with it. One queued navigation presented as a page where
nothing works.

Two fixes, and both were needed:

* **`pagehide` closes the source and `pageshow` reopens it.** Not `visibilitychange`, which fires
  when the window merely loses focus. A queue left up on a second screen is a real use, and closing
  the stream there would freeze a page somebody is looking at. Not `unload` either. Registering for
  it disqualifies the page from the back/forward cache in every current browser. That frees the
  connection by deleting the feature it is competing with.
* **These files are cached now**, `public, max-age=31536000, immutable`, with a `?v=` stamp on every
  URL in `layout.html`. They went out with a content type and nothing else. A browser therefore had
  no freshness information, and re-fetched every one of them on every navigation. That is four of the
  six connections spent before the page has asked for anything.

**Closing is only safe because the server replays.** `Hub::stream` sends the latest of every state
event to a stream as it opens. A page coming back out of the cache is therefore brought up to date by
the same mechanism that fills one on its first load. The reopen needed nothing added. Remove that
replay, though, and this stops being free: the two have to move together.

### …and `visibilitychange` is the right event to *wake* on

The bullet above is true about *closing* and says nothing about waking. **The Android remote's
Activity stopping is not a navigation.** Neither `pagehide` nor `pageshow` fires, so nothing closes
or reopens the stream and no replay happens on the way back. The page keeps whatever fragment it was
last pushed, which after a background is a red banner for a machine that has since come back. Nothing
corrects it, because the pump republishes the connection only when it *changes*.

Pressing a tab appears to fix it, which is what sends the diagnosis to the wrong place. A tab press
is an ordinary link, so it replaces the document and renders the banner from the connection as it is
now. The fix is to make coming back on screen do what a tab press already does.

`live.js` therefore takes a **fresh** stream when the page becomes visible: close then open, so the
replay runs. It never closes on `hidden`, which leaves the second-screen case above exactly as it
was. Three details, each of which would be a fault on its own:

* **A reopen is at most one a second** (`REOPEN_MIN_GAP_MS`). A restore from the back/forward cache
  fires `pageshow` *and* `visibilitychange`, and a notification shade can flap visibility twice in a
  moment. Without the floor, each one aborts a request that had just gone out.
* **`live.js` listens for the `resume` event too.** That is the Page Lifecycle spelling of the same
  moment, for a browser that froze the page rather than merely hiding it. One idle listener where it
  is not implemented.
* **The reopen is also the retry.** `GET /events` treats a stream opening against an absent machine
  as a request to try it now. The reopen therefore fetches a new answer rather than refreshing the
  page's copy of a stale one. That is why this is one mechanism rather than two.

This is the shim the table in `docs/architecture/remote.md` says the Go remote needed and we did not.
Half of that row was right. `EventSource` does reconnect on its own and the hub does replay, so
nothing here reproduces the extension's connection handling. What it missed is that a WebView
background fires no event either of those hangs off.

As with both sections above, **nothing in this repository could have caught it**. `tests/pages.rs`
drives the router as a service, so there is no browser, no navigation and no connection pool anywhere
in the loop. One assertion guards the two listeners being still in this file
(`the_live_script_releases_its_stream_when_the_page_leaves`). Two real tests cover the headers and
the stamped URLs, which are the halves a router can observe.

## The scroll position is reconstructed, because a tab press has nothing to preserve

The same fact as the section above, read the other way round. A tab press replaces the document, so
coming back to the Songs tab would otherwise mean coming back to the top of the corpus. Boosting the
tab bar would keep the scroll for nothing, and it is refused: two decisions depend on a disclosure
*not* surviving a tab press. So `live.js` writes down where somebody was, and finds it again.

* **On `pagehide`, the first row whose *top* has reached the sticky bar's bottom edge** is written
  into `km_at` as `row=<code>&at=<index>&list=<tag>`. `pagehide` and not `visibilitychange` — that is
  this file's standing rule and the reason is two sections up. `document.cookie` because it is a
  synchronous write that lands before the navigation, where a request would be racing it.
* **The selection rule and the restore have to be each other's inverse**, and the obvious spelling is
  not. Taking the first row merely *overlapping* that edge picks a row that clears it by 0.45 of a
  pixel, and is therefore entirely hidden. Measured: `bottom` 137.98 against a bar bottom of 137.53. Restoring it put it flush below the bar and moved the list down a row, so the next
  capture took the row above. That is one row of drift per round trip. Selecting on the *top* is a
  fixed point of the restore.
* **A cookie rather than browser storage, and the reason is not idiom.** The server has to know
  before it renders. Rows past the first fifty exist only because `Load more` appended them, and
  nothing server-side records that. An anchor the server cannot see would mean the script pressing
  that button three times, visibly, under a document whose height is changing.
  `handlers::browse` reads the cookie and renders straight to the page that holds the row.
* **A row and never a pixel.** Rows wrap to two lines, and the sticky bar's height differs between
  the document you left and the one you came back to. The half that decides it: a pixel that misses
  still lands somewhere, and past the end of a shorter document the browser clamps it. That clamp is
  the exact fault the ⋯ toggle was changed to stop causing. A row that is not found scrolls nothing,
  and nothing is the top of the list.
* **The list tag is what stops an anchor reaching the wrong list.** A search swaps `#list` without a
  navigation, so an anchor can outlive the list it was taken in. `km_browse` cannot serve as the
  check, because that same swap rewrites it.

**This is the one cookie the browser writes and the server never does.** That is why `prefs::set` can
keep `HttpOnly` unconditional, and `km_access` is why that matters. As with everything else in this
file, no test here has a browser in it. The guard is that the listeners and the two `data-`
attributes stay where the other half expects them.
`the_live_script_remembers_which_row_was_on_screen` and
`the_live_script_scrolls_to_a_row_and_never_to_a_pixel` hold that, and `tests/pages.rs` takes the
server half end to end.

## `hx-vals` is inherited, and `hx-disinherit` does not stop it

The second thing about this vendored copy that surprised us, and it cost two buttons rather than
thirteen. Measured against `htmx.min.js` at 2.0.4; check it again on the next bump.

* **A parent walk collects `hx-vals`, using a plain `getAttribute`.** `Sn(e,t)` is
  `bn(e,"hx-vals",false,t)`, and `bn` ends `return bn(ue(c(r)),e,o,i)` where `c` is `parentElement`.
  A child's own keys win; an ancestor's fill the gaps.
* **`hx-disinherit` never reaches it.** That lookup is a different pair of functions, and `hx-vals`
  occurs exactly once in the whole file. Putting `hx-disinherit="hx-vals"` on a parent looks like a
  fix, changes nothing, and leaves a comment claiming otherwise. The only escape htmx offers is the
  literal `hx-vals="unset"` on the descendant itself.
* **For a GET the collected values are appended to the `hx-get` URL, existing query string and all**
  — `if(R.indexOf("?")<0){R+="?"}else{R+="&"}R+=an(w)` — with no de-duplication.

What that cost: `_browse.html`'s search form carried `hx-vals='{"fragment": "list"}'`, and the ✕ and
⋯ buttons written inside it each named `fragment=browse` in their own `hx-get`. Every press asked for
the key twice. `axum::Query` over a derived struct answers a duplicate key with 400, and htmx does
not swap on a non-2xx. Both were inert from the commit that added them.

**And, exactly as with the SSE debt above, nothing caught it.** `tests/pages.rs` drives the router as
a service, so the URL a test types is the URL the handler sees. The URL a browser builds is the one
nobody wrote down. The guard is now an assertion on the rendered attributes rather than on a request:
`the_search_form_carries_nothing_its_buttons_can_inherit`.

## `revealed` costs two listeners and a timer, and `intersect` costs a constructor

What the `Load more` button's automatic trigger rests on. Measured against `htmx.min.js` at 2.0.4;
check it again on the next bump.

* **`revealed` is a scroll listener, a resize listener and a 200ms `setInterval`**, shared by every
  element that asks for it. The interval does nothing until one of the two listeners sets a flag. A
  layout that changes without a scroll therefore costs a boolean test five times a second and no DOM
  work. With the flag set, it runs one `querySelectorAll` for `[hx-trigger*='revealed']` over the
  whole document. htmx installs all three on first use and never tears them down. A list with nothing
  more to fetch draws no button, and starts neither the listeners nor the timer.
* **It fires once per element.** `bt` stamps `data-hx-revealed` before it triggers, and the swapped-in
  page brings its own unstamped button. A fire that is swallowed downstream is therefore a fire that
  does not come back.
* **It fires immediately if the element is already on screen**, because binding ends with a check
  rather than waiting for a first scroll.
* **`intersect` constructs its `IntersectionObserver` before it binds anything**, and neither `St`,
  `wt` nor `Nt` wraps that call. A browser without the constructor loses the entire `htmx.process` of
  whatever was swapped in, not the one control that asked for the trigger. It also reads `root` and
  `threshold` and no `rootMargin`, so it cannot ask for a page early.
* **Nothing inherits `hx-trigger`.** `st(e)` reads it with `te(e,"hx-trigger")`, a plain
  `getAttribute` on that element with no parent walk. It is the opposite of `hx-vals` above and needs
  no `unset` escape. A `<button>` with the attribute present loses the implicit `click` it would
  otherwise fall through to, which is why `click` is written out beside `revealed`.

The guard is an assertion on the rendered attributes, for the reason the row above gives:
`the_next_page_is_asked_for_by_being_scrolled_to` and `a_list_with_no_next_page_is_not_scrolled_to`.
