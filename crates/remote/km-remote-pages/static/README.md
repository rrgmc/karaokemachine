# Vendored assets

**Compiled into whichever binary links this crate** by `include_str!` in `src/lib.rs`, which serves
them at `/static/`. They are not shipped as files and nothing stages this folder — see the *Bundling
assets* entry in `docs/decisions/`. Editing a file here changes the next build; there is nothing to copy
anywhere afterwards.

That rule matters more here than it does for the curation tool. This crate ends up inside the
appliance's `.deb`, inside a macOS bundle, inside a portable folder and inside `km-remote`, and a
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

**jsQR is Apache-2.0, not MIT**, and the license text is shipped because that license requires it —
which is the same treatment htmx gets and the reason both `-LICENSE.txt` files are routed at
`/static/` rather than merely sitting here. The copy in the Go remote this crate follows carries no
license header and its own asset list records none, so the license was taken from upstream rather
than from that copy.

**A quarter of a megabyte, and it lands in the machine's binary too.** This crate is linked by
`karaokemachine` as well as by `km-remote`, and `include_str!` is unconditional, so the appliance
carries a decoder for a page its own remote never draws. That is the accepted cost: a cargo feature
would make this the first *compile-time* mode split in a crate whose header says the templates branch
on capabilities and never on a mode, and the `Bundling assets` decision admits an exception only for
size on the order of the 31 MiB SoundFont or for a license that forbids redistribution. Neither
applies. It is loaded by `share_receive.html` alone rather than from the shared head, so no other
page parses it.

The htmx here is the same version `tools/cmd/km-package-builder` vendors, deliberately: two copies of one library
at two versions is a difference nobody would look for when a swap behaves oddly in one tool and not
the other. Update both together.

```sh
curl -sSL -o htmx.min.js https://unpkg.com/htmx.org@<version>/dist/htmx.min.js
curl -sSL -o htmx-LICENSE.txt https://raw.githubusercontent.com/bigskysoftware/htmx/v<version>/LICENSE

curl -sSL -o jsqr.js https://unpkg.com/jsqr@<version>/dist/jsQR.js
curl -sSL -o jsqr-LICENSE.txt https://raw.githubusercontent.com/cozmo/jsQR/master/LICENSE
```

jsQR's UMD bundle assigns `window.jsQR`, which is the name `scan.js` tests for and calls. Checking
that after an update is worth a moment: the wrapper prefers a CommonJS `exports` when it finds one,
so loading the file under Node to confirm it "works" can take a branch a browser never will.

## Why htmx's SSE extension is not vendored

htmx ships server-sent events as a separate `ext/sse.js`, and `live.js` — about thirty lines around
the browser's own `EventSource` — does the job instead. Three reasons, and the third is the one that
decided it.

* `EventSource` already reconnects by itself. The extension layers its own connection handling on
  top, and in the Go remote this crate follows, getting that to re-register its swap targets after an
  iOS suspend needed a shim of its own. Not reproducing the extension means not reproducing the shim.
* A second vendored file has a second version to keep in step with this one.
* The extension's main draw is `hx-preserve`, which keeps a control from being destroyed while a
  finger is on it when a fragment is replaced underneath. **The remote does not need it, because
  nothing replaces a control four times a second.** The machine's `state` event arrives every 250 ms,
  but the pump splits what it carries: the player *card* — every button, stepper and slider — is
  republished only when the song or the settings actually change, while the part that moves
  constantly is a separate `position` fragment holding the elapsed time and the bar, and at most one
  a second. Solving it at the source rather than guarding against it is the difference between a
  control that is never rebuilt under a finger and one that is rebuilt and put back.

**The one thing not vendoring the SSE extension costs.** That extension calls `htmx.process()` on
everything it swaps in; `live.js` has to do that itself. htmx binds `hx-*` triggers only in
`htmx.process()` — once over `document.body` at
`DOMContentLoaded`, and again over whatever htmx swaps itself — and the vendored 2.0.4 carries no
`MutationObserver`, so a node some other script inserts is live DOM that htmx has never seen. It
renders correctly and every control in it is inert.

That is not a theoretical hazard: `_player.html` is thirteen `hx-post` attributes inside the element
wearing `data-sse="player"`, and `Hub::stream` replays the latest `player` frame to every stream as
it opens. So the server-rendered card — which htmx *had* processed — is replaced by an identical dead
one within milliseconds of the page loading, and the Now tab's buttons never work for
anybody. `_queue.html` had the same defect for the same reason. Nothing caught it because
`tests/pages.rs` drives the router as a service and asserts on HTML strings; there is no browser in
the loop, so htmx binding is never exercised.

**So the rule for `live.js` is: everything it inserts gets processed.** `swap` builds the fragment in
a `<template>` and `replaceWith`s it precisely so there is a node left to hand to `htmx.process` —
an `outerHTML` assignment parses the nodes into the document and leaves no reference to them.

## The stream is closed when the page leaves the screen, and reopened when it comes back

The third thing about a browser that had to be learned here rather than guessed, and the most
expensive of the three: it presented as *the machine* being slow.

`live.js` opened one `EventSource` and left it open. That reads as obviously right — the page wants
the stream for as long as it exists — and it is wrong for a reason that is nowhere in this file: the
three tabs in `layout.html` are ordinary links, so a tab press does not keep the page, it replaces
it. The page that was replaced is not destroyed either; the browser keeps it in its back/forward
cache, holding its connection, where nobody can see it and nothing will ever close it.

A browser allows about **six connections to one host**. A navigation wanted six of its own — the
HTML, the four files in this folder, and a new stream — so two or three abandoned streams and the
next request had nothing to ask with. It queued until the browser evicted a cached page, which is
tens of seconds. And `hx-sync="body:queue last"` on the body allows the document one in-flight
request, so the first thing to queue stopped every later tap with it: one queued navigation
presenting as a page where nothing works.

Two fixes, and both were needed:

* **`pagehide` closes the source and `pageshow` reopens it.** Not `visibilitychange`, which fires
  when the window merely loses focus — a queue left up on a second screen is a real use, and closing
  the stream there would freeze a page somebody is looking at. Not `unload` either: registering for
  it disqualifies the page from the back/forward cache in every current browser, which frees the
  connection by deleting the feature it is competing with.
* **These files are cached now**, `public, max-age=31536000, immutable`, with a `?v=` stamp on every
  URL in `layout.html`. They were served with a content type and nothing else, so a browser had no
  freshness information at all and re-fetched every one of them on every navigation. That is four of
  the six connections spent before the page has asked for anything.

**Closing is only safe because the server replays.** `Hub::stream` sends the latest of every state
event to a stream as it opens, so a page coming back out of the cache is brought up to date by the
same mechanism that fills one on its first load. Nothing was added for the reopen — but if that
replay is ever removed, this stops being free and the two have to move together.

### …and `visibilitychange` is the right event to *wake* on

The bullet above is true about *closing* and says nothing about waking. **The Android remote's
Activity stopping is not a navigation**: neither
`pagehide` nor `pageshow` fires, so the stream is neither closed nor reopened and no replay happens
on the way back. The page keeps whatever fragment it was last pushed — after a background, a red
banner for a machine that has since come back — and nothing corrects it, because the pump
republishes the connection only when it *changes*.

Pressing a tab appears to fix it, which is what sends the diagnosis to the wrong place: a tab press
is an ordinary link, so it replaces the document and renders the banner from the connection as it is
now. The fix is to make coming back on screen do what a tab press already does.

`live.js` therefore takes a **fresh** stream when the page becomes visible — close then open, so the
replay runs — and never closes on `hidden`, which leaves the second-screen case above exactly as it
was. Three details, each of which would be a fault on its own:

* **A reopen is at most one a second** (`REOPEN_MIN_GAP_MS`). A restore from the back/forward cache
  fires `pageshow` *and* `visibilitychange`, and a notification shade can flap visibility twice in a
  moment; without the floor each one aborts a request that had just gone out.
* **The `resume` event is listened for too**, which is the Page Lifecycle spelling of the same moment
  for a browser that froze the page rather than merely hiding it. One idle listener where it is not
  implemented.
* **The reopen is also the retry.** `GET /events` treats a stream opening against an absent machine
  as a request to try it now, so the reopen does not merely refresh the page's copy of a stale
  answer — it goes and gets a new one. That is why this is one mechanism rather than two.

This is the shim the table in `docs/architecture/remote.md` says the Go remote needed and we did
not. Half of that row was right: `EventSource` does reconnect on its own and the hub does replay, so
none of the extension's connection handling is reproduced here. What it missed is that a WebView
background fires no event either of those hangs off.

And as with both sections above, **nothing in this repository could have caught it**: `tests/pages.rs`
drives the router as a service, so there is no browser, no navigation and no connection pool anywhere
in the loop. What guards it is an assertion that the two listeners are still in this file
(`the_live_script_releases_its_stream_when_the_page_leaves`) and two real tests over the headers and
the stamped URLs, which are the halves that *are* observable from a router.

## The scroll position is reconstructed, because a tab press has nothing to preserve

The same fact as the section above, read the other way round: a tab press replaces the document, so
coming back to the Songs tab would otherwise mean coming back to the top of the corpus. Boosting the
tab bar would keep the scroll for nothing, and is refused — two decisions depend on a disclosure
*not* surviving a tab press. So `live.js` writes down where somebody was and finds it again.

* **On `pagehide`, the first row whose *top* has reached the sticky bar's bottom edge** is written
  into `km_at` as `row=<code>&at=<index>&list=<tag>`. `pagehide` and not `visibilitychange` — that is
  this file's standing rule and the reason is two sections up. `document.cookie` because it is a
  synchronous write that lands before the navigation, where a request would be racing it.
* **The selection rule and the restore have to be each other's inverse**, and the obvious spelling is
  not. Taking the first row merely *overlapping* that edge picks a row that can be clearing it by
  0.45 of a pixel — measured, `bottom` 137.98 against a bar bottom of 137.53 — and is therefore
  entirely hidden. Restoring it put it flush below the bar and moved the list down a row, so the next
  capture took the row above: one row of drift per round trip. Selecting on the *top* is a fixed
  point of the restore.
* **A cookie rather than browser storage, and the reason is not idiom.** The server has to know
  before it renders: rows past the first fifty exist only because `Load more` appended them and
  nothing server-side records that, so an anchor the server cannot see would mean the script driving
  that button three times, visibly, while trying to set a scroll under a document whose height is
  changing. `handlers::browse` reads the cookie and renders straight to the page that holds the row.
* **A row and never a pixel.** Rows wrap to two lines, the sticky bar's height differs between the
  document you left and the one you came back to, and — the half that decides it — a pixel that
  misses still lands somewhere, and past the end of a shorter document the browser clamps it. That
  clamp is the exact fault the ⋯ toggle was changed to stop causing. A row that is not found scrolls
  nothing, and nothing is the top of the list.
* **The list tag is what stops an anchor being applied to the wrong list.** A search swaps `#list`
  without a navigation, so an anchor can outlive the list it was taken in; `km_browse` cannot serve
  as the check because that same swap rewrites it.

**This is the one cookie the browser writes and the server never does**, which is why `prefs::set`
can keep `HttpOnly` unconditional — `km_token` is why that matters. And as with everything else in
this file, no test here has a browser in it: what is guarded is that the listeners and the two
`data-` attributes are still where the other half expects them
(`the_live_script_remembers_which_row_was_on_screen`,
`the_live_script_scrolls_to_a_row_and_never_to_a_pixel`), plus the server half end to end in
`tests/pages.rs`.

## `hx-vals` is inherited, and `hx-disinherit` does not stop it

The second thing about this vendored copy that surprised us, and it cost two buttons rather than
thirteen. Measured against `htmx.min.js` at 2.0.4; check it again on the next bump.

* **`hx-vals` is collected by a parent walk using a plain `getAttribute`.** `Sn(e,t)` is
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
the key twice, `axum::Query` over a derived struct answers a duplicate key with 400, and htmx does
not swap on a non-2xx. Both were inert from the commit that added them.

**And, exactly as with the SSE debt above, nothing caught it**: `tests/pages.rs` drives the router as
a service, so the URL a test types is the URL the handler sees, and the URL a browser builds is the
one nobody wrote down. The guard is now an assertion on the rendered attributes rather than on a
request — `the_search_form_carries_nothing_its_buttons_can_inherit`.

## `revealed` costs two listeners and a timer, and `intersect` costs a constructor

What the `Load more` button's automatic trigger rests on. Measured against `htmx.min.js` at 2.0.4;
check it again on the next bump.

* **`revealed` is a scroll listener, a resize listener and a 200ms `setInterval`**, shared by every
  element that asks for it. The interval does nothing until one of the two listeners sets a flag, so
  a layout that changes without a scroll costs a boolean test five times a second and no DOM work.
  When the flag is set it runs one `querySelectorAll` for `[hx-trigger*='revealed']` over the whole
  document. Installed on first use and never torn down, so a list with nothing more to fetch, which
  draws no button, starts neither the listeners nor the timer.
* **It fires once per element.** `bt` stamps `data-hx-revealed` before it triggers, and the swapped-in
  page brings its own unstamped button. A fire that is swallowed downstream is therefore a fire that
  does not come back.
* **It fires immediately if the element is already on screen**, because binding ends with a check
  rather than waiting for a first scroll.
* **`intersect` constructs its `IntersectionObserver` before it binds anything**, and neither `St`,
  `wt` nor `Nt` wraps that call. A browser without the constructor loses the entire `htmx.process` of
  whatever was swapped in, not the one control that asked for the trigger. It also reads `root` and
  `threshold` and no `rootMargin`, so it cannot ask for a page early.
* **`hx-trigger` is not inherited.** `st(e)` reads it with `te(e,"hx-trigger")`, which is a plain
  `getAttribute` on that element and no parent walk, so it is the opposite of `hx-vals` above and
  needs no `unset` escape. A `<button>` with the attribute present loses the implicit `click` it
  would otherwise fall through to, which is why `click` is written out beside `revealed`.

The guard is an assertion on the rendered attributes, for the reason the row above gives:
`the_next_page_is_asked_for_by_being_scrolled_to` and `a_list_with_no_next_page_is_not_scrolled_to`.
