// Keeps the page in step with the machine.
//
// One EventSource for the whole document. Each named event carries a rendered HTML fragment, and the
// element wearing the matching `data-sse` attribute is replaced by it. That is the entire mechanism:
// no client-side model, no templating in the browser, and every fragment produced by the same askama
// template that produced it during the full-page render, so the two can never drift.
//
// `EventSource` rather than htmx's SSE extension. It reconnects by itself with a backoff the browser
// owns, which is the part the extension re-implements and the part that needed a shim in the Go
// remote this follows. See `static/README.md`.

(() => {
  "use strict";

  // Replaced wholesale, so the fragment must carry the same `data-sse` attribute it is replacing.
  // Re-querying on every event rather than caching the node is what makes that work: a replaced node
  // is detached, and a cached reference would update nothing, silently, for the rest of the session.
  // `machine` is offline-only and the online remote never publishes it, so listening for it there
  // costs one idle listener and no markup — cheaper than a build-time split of this file.
  // `nowbar` is the Queue tab's twin of `player` — the same state, a smaller fragment. Like
  // `machine`, an event a given page may have no target for costs one idle listener and no markup,
  // which is cheaper than a build-time split of this file.
  const STATE_EVENTS = [
    "player",
    "position",
    "nowbar",
    "queue",
    "queuecount",
    "conn",
    "banner",
    "machine",
  ];

  // How long a toast stays. Must match the `toast` keyframes in app.css: the element is removed on
  // `animationend`, and this is only the fallback for a browser that never fires one — a tab in the
  // background, or `prefers-reduced-motion` shortening the animation to nothing.
  const TOAST_FALLBACK_MS = 6000;

  // How long the offline banner stays on screen before it collapses.
  //
  // The machine being away is the normal case — the television is off most of the day — so a red
  // strip across the top of every page for the whole evening is a nag rather than news. It says its
  // piece and goes; the tab-bar dot and the Now tab's machine card are the standing indicators, and
  // both report exactly the same thing. A *changed* reason shows again, because the server sends a
  // fresh element and the timer below is armed per element.
  const BANNER_LINGER_MS = 8000;

  // ...and how long it says only that it is trying, before it is willing to say it failed. The
  // strip arrives wearing `banner-trying`; this is when that comes off. Its eight seconds of red
  // then start, so a real outage is reported for exactly as long as it always was.
  //
  // **What it has to cover is one recovery, measured from the moment the strip is published** —
  // which is already after the machine has been silent for `STREAM_IDLE_TIMEOUT`. That is one
  // `BACKOFF_START` of waiting, plus a handshake on a home network, plus up to one
  // `CONNECTION_INTERVAL` for the pump to notice the connection came back and publish the empty
  // banner over the top of this one. Two seconds and a bit, generously rounded up, and doubled
  // again for a phone whose radio has only just woken.
  //
  // So it is coupled to the *retry* interval and not to the detection deadline: making the remote
  // slower to notice silence would not need this number to move, and making it slower to retry
  // would.
  const BANNER_GRACE_MS = 5000;

  // Hands a freshly inserted subtree to htmx so its `hx-*` attributes are bound.
  //
  // **This is not optional, and leaving it out is invisible until somebody presses a button.** htmx
  // binds triggers only in `htmx.process()` — once over `document.body` at DOMContentLoaded, and
  // again over whatever it swaps itself. It has no MutationObserver, so a node this file inserts is
  // live DOM that htmx has never seen: it renders correctly, it looks right, and every control in it
  // is inert. The player card is the case that bit — `_player.html` is thirteen `hx-post`
  // attributes, and the hub replays the latest `player` frame to every stream that opens, so the
  // server-rendered card was replaced by an identical dead one within milliseconds of the page
  // loading and the Now tab's buttons had never worked.
  function bind(node) {
    if (window.htmx) window.htmx.process(node);
  }

  // `replaceWith` through a `<template>` rather than `target.outerHTML = html`, because the
  // replacement has to be *reachable* to be processed: after an `outerHTML` assignment the parsed
  // nodes are in the document and the only reference to them is gone. Building them first gives a
  // handle to hand to `bind`.
  // Starts the offline banner's clock.
  //
  // **Hidden, never removed.** Unlike a toast or a badge, this element is an SSE swap target, and
  // `_banner.html` renders an empty one rather than nothing precisely so the next event has
  // somewhere to land. `display: none` leaves it in the DOM where `swap` can still find it.
  //
  // Driven by a timer rather than by a CSS animation, which is the mistake the two `*_FALLBACK_MS`
  // constants above exist because of: an animation can be shortened to nothing by
  // `prefers-reduced-motion`, and `animationend` need never fire in a backgrounded tab. A bar that
  // needed an animation in order to leave would be the fault it was meant to fix, wearing a hat.
  //
  // Two phases, chained rather than scheduled from one instant: the linger is measured from when
  // the strip actually turned red, so neither number has to know the other and there is no third
  // constant that is the sum of them.
  //
  // A phase left pending on an element the next event replaces fires on a detached node and does
  // nothing, which is what makes the whole of this safe across a background — the frozen clock
  // resumes, finds itself holding a node nobody can see, and stops mattering.
  function armBanner(node) {
    if (!(node instanceof Element) || !node.classList.contains("banner")) return;
    setTimeout(() => {
      node.classList.remove("banner-trying");
      setTimeout(() => node.classList.add("spent"), BANNER_LINGER_MS);
    }, BANNER_GRACE_MS);
  }

  function swap(name, html) {
    for (const target of document.querySelectorAll(`[data-sse="${name}"]`)) {
      const template = document.createElement("template");
      template.innerHTML = html.trim();
      // A fragment is normally one element, but nothing guarantees it, and `replaceWith` empties the
      // template's content — so the list is taken before the swap and used after it.
      const fresh = Array.from(template.content.children);
      if (fresh.length === 0) continue;
      target.replaceWith(template.content);
      for (const node of fresh) {
        bind(node);
        if (name === "banner") armBanner(node);
      }
    }
  }

  function toast(html) {
    const tray = document.getElementById("toasts");
    if (!tray) return;
    tray.insertAdjacentHTML("afterbegin", html);
    const item = tray.firstElementChild;
    if (!item) return;
    // `_toast.html` carries no `hx-*` today, so this binds nothing. It is here because the rule is
    // "everything this file inserts gets processed" — a toast that one day grows an Undo button
    // should not have to rediscover the defect above.
    bind(item);
    const drop = () => item.remove();
    item.addEventListener("animationend", drop, { once: true });
    setTimeout(drop, TOAST_FALLBACK_MS);
  }

  // ------------------------------------------------------------------ badges

  // **A row's badge has to be removed, where a toast only has to fade.** `app.css` hides a row's
  // buttons with `.song-actions:has(.badge.flash)`, so the span going invisible at the end of its
  // animation is not enough — while it is still in the DOM the buttons it stands in front of stay
  // hidden, which is the whole fault this replaced. A leftover toast is invisible and inert; a
  // leftover badge is a row nobody can press again.
  //
  // Delegated from `document` rather than attached per element, because unlike a toast this file
  // does not insert it: the badge arrives in htmx's own swap of `POST /song/{number}/{action}`, so
  // there is no moment here that holds the node. `animationend` bubbles, which is what makes that
  // work.
  document.addEventListener("animationend", (event) => {
    const badge = event.target;
    if (badge instanceof Element && badge.classList.contains("flash")) badge.remove();
  });

  // The same fallback the toast has, for the same reason and one more. A browser that never fires
  // `animationend` — a backgrounded tab, an animation switched off somewhere this file cannot see —
  // would stop the listener above ever running, and here that costs a row rather than a message.
  // Longer than the 3s the `flash` keyframes run for, so it never beats them to it.
  const BADGE_FALLBACK_MS = 6000;

  document.addEventListener("htmx:afterSwap", (event) => {
    const target = event.detail && event.detail.target;
    if (!(target instanceof Element)) return;
    const badges = target.querySelectorAll(".badge.flash");

    // **Two taps really do produce two badges, and `hx-sync` is not what stops it.**
    // `body:queue last` holds the second request and sends it once the first is answered — it queues
    // rather than drops — so a double tap on a phone is answered twice and stacks. Only the newest
    // is kept, which with `afterbegin` is the first.
    //
    // **Removed, never hidden.** A hidden element runs no animation, so it would fire no
    // `animationend`, and `:has` would go on matching a badge nobody can see — the row's buttons
    // hidden for the rest of the evening, which is the fault all of this replaced wearing a hat. The
    // timer already armed for a removed badge calls `.remove()` on a detached node, which does
    // nothing.
    for (let index = 1; index < badges.length; index += 1) badges[index].remove();

    if (badges[0]) setTimeout(() => badges[0].remove(), BADGE_FALLBACK_MS);
  });

  // **One stream, and only while this page is on screen.** The three tabs are ordinary links, so
  // every tab press replaces the document — and a document that opened a stream and never closed it
  // leaves the socket held while the browser keeps the page in its back/forward cache. A browser
  // allows about six connections per host, so two or three abandoned streams and the *next*
  // navigation has nothing left to ask with: its requests sit in the queue until something frees a
  // socket, which for a cached page is when the browser gets round to evicting it. That is a stall
  // of tens of seconds on a page that is doing nothing, and `hx-sync="body:queue last"` in
  // `layout.html` then spreads it to every later tap, one in-flight request being all the document
  // is allowed.
  //
  // Reopening costs nothing to get right, which is what makes closing safe: the hub replays the
  // latest of every state event to a stream as it opens (`REPLAYED` in `src/sse.rs`), so a page
  // coming back is brought up to date by the same mechanism that fills one on its first load.
  let source = null;

  // When the stream was last opened. Two things fire on a restore from the back/forward cache —
  // `pageshow` and `visibilitychange` — and a notification shade or a permission prompt can flap
  // visibility twice in a moment; without this each one would abort a request that had just gone
  // out and ask the machine to retry again. One reopen a second is plenty for a signal that means
  // "somebody is looking at this".
  const REOPEN_MIN_GAP_MS = 1000;
  let openedAt = 0;

  function open() {
    if (source) return;
    openedAt = Date.now();
    source = new EventSource("/events");
    for (const name of STATE_EVENTS) {
      source.addEventListener(name, (event) => swap(name, event.data));
    }
    source.addEventListener("toast", (event) => toast(event.data));

    // A dropped stream is not worth telling anybody about: the browser is already reconnecting, and
    // the machine being unreachable is reported by the banner, which is itself one of these events.
    // Saying it twice, in two places, with two different recovery stories, is how a remote that is
    // merely between reconnects comes to look broken.
    source.addEventListener("error", () => {
      if (source && source.readyState === EventSource.CLOSED) {
        console.warn("[km-remote-pages] event stream closed");
      }
    });
  }

  function close() {
    if (!source) return;
    source.close();
    source = null;
  }

  // **The page is in front of somebody again: take a fresh stream.** Not a repair of a stream that
  // is known to be broken — one that *looks* open is the case this is for.
  //
  // Two things happen on the reopen and both are needed. The server replays the latest of every
  // state event to a stream as it opens, so a page that missed a change while nobody was reading is
  // brought up to date; and the handler treats a stream opening against an absent machine as a
  // request to try it now, so the remote stops waiting out a backoff it measured before the phone
  // went into a pocket.
  function wake() {
    if (Date.now() - openedAt < REOPEN_MIN_GAP_MS) return;
    close();
    open();
  }

  open();

  // The banner that came with the page, as against the ones `swap` will insert. `defer` puts this
  // after the document is parsed, so the element is already there.
  for (const node of document.querySelectorAll('[data-sse="banner"]')) armBanner(node);

  // **`visibilitychange` is the wrong event to close on and the right one to wake on**, and the
  // asymmetry is the whole of this block.
  //
  // *Closing* belongs to `pagehide`/`pageshow`, which fire when the document stops and starts being
  // the one the window is showing — a navigation away, a back button, a restore from the cache —
  // which is exactly when the socket is and is not wanted. Closing on `visibilitychange` would
  // close it when the window merely lost focus, and a queue left up on a second screen while
  // somebody works in another window is a thing people actually do. `unload` is the older spelling
  // and is wrong for a third reason: registering for it disqualifies the page from the back/forward
  // cache in every current browser, which would free the socket by deleting the feature the socket
  // is competing with.
  //
  // *Waking* has no other event available, and the Android remote is why it is needed. That app is
  // a WebView on a server inside the same process, and its Activity stopping is **not a
  // navigation** — so `pagehide` and `pageshow` never fire, the stream is neither closed nor
  // reopened, and no replay happens on the way back. The page simply keeps whatever fragment it
  // was last pushed, which after a background is a red banner for a machine that has since come
  // back. Pressing a tab appeared to fix it because a tab press is an ordinary link: it replaces
  // the document and re-renders the banner from the connection as it is now. That, and not the
  // backoff, is why switching tabs "notices almost immediately".
  //
  // Nothing here closes on `hidden`, so the second-screen case above is untouched: this only ever
  // adds a reopen, and `wake` will not do even that twice in a second.
  window.addEventListener("pagehide", close);
  window.addEventListener("pageshow", open);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") wake();
  });
  // The Page Lifecycle spelling of the same moment, for a browser that froze the page rather than
  // merely hiding it. Costs one idle listener where it is not implemented.
  document.addEventListener("resume", wake);

  // ---- Coming back to the row you were reading -------------------------------------------------
  //
  // Every tab in `layout.html` is an ordinary link, deliberately — two shipped decisions depend on
  // a tab press replacing the document, so a disclosure cannot survive one. That means the scroll
  // is *reconstructed*, not preserved, and the two halves below are the reconstruction: one writes
  // down which row was on screen, the other finds it again in the document the server sent back.
  //
  // **A row and never a pixel.** Rows are not a fixed height (`.song-name` wraps), the sticky bar
  // above them is not a fixed height either, and — the part that decides it — a pixel offset cannot
  // fail safely: a saved pixel that misses still lands *somewhere*, and past the end of a document
  // that came back shorter the browser clamps it, which is precisely the fault the ⋯ toggle was
  // changed to stop causing. A row that cannot be found scrolls nothing, and nothing is the top of
  // the list, which is what this tab did before any of this existed.

  // The cookie, and the hour it lives for, are `prefs::AT`'s. This is the only cookie the browser
  // writes and the only one the server does not; `HttpOnly` is unconditional on that side because
  // of `km_access`, so this one is set here or nowhere.
  const AT_COOKIE = "km_at";
  const AT_MAX_AGE = 3600;

  // The rows of the outermost list. `Load more` swaps a whole `_rows.html` in, so a document that
  // has loaded four pages holds four elements with `id="rows"` nested inside one another —
  // `getElementById` gives the outer one, which is where the server stamps the restore, and a
  // descendant query then reaches every row in all four.
  const rowsIn = (list) => list.querySelectorAll("li.song-row[data-row]");

  // The bar is measured rather than declared: `.browse-bar` is sticky with no set height, and it
  // genuinely differs between the document you left and the one you came back to — the banner, an
  // artist's heading and the filter row each change it.
  const barBottom = () => {
    const bar = document.querySelector(".browse-bar");
    return bar ? bar.getBoundingClientRect().bottom : 0;
  };

  // `pagehide` and only `pagehide`. The page becoming hidden must not be acted on — that is this
  // file's standing rule, and the reason is two windows up. `document.cookie` is a synchronous
  // write, so it lands before the navigation in a way no request could be relied on to.
  window.addEventListener("pagehide", () => {
    const list = document.getElementById("rows");
    if (!list) return; // Not the Songs tab.
    // The first row whose *top* has reached the bar's bottom edge — the first one printed below the
    // furniture rather than behind it. Measured against the bar and not the viewport because the bar
    // is what is covering the list.
    //
    // **The rule has to be the one the restore below re-establishes, or the two drift.** Taking the
    // first row merely *overlapping* the line reads as the obvious spelling and is wrong: a row can
    // clear that edge by half a pixel and be, for every purpose, entirely hidden. Restoring it then
    // puts it flush under the bar and moves the whole list down a row, so the next capture takes the
    // row above — measured drifting exactly one row per round trip. Aligning a row's top to the line
    // and then selecting on that same top is a fixed point, which is what makes leaving and coming
    // back twice land in the same place as doing it once. `TOP_SLACK` is for subpixel layout only.
    const TOP_SLACK = 1;
    const line = barBottom();
    const rows = rowsIn(list);
    let index = -1;
    let code = "";
    for (let i = 0; i < rows.length; i += 1) {
      if (rows[i].getBoundingClientRect().top >= line - TOP_SLACK) {
        index = i;
        code = rows[i].dataset.row;
        break;
      }
    }
    // At the top there is nothing to remember, and a stale anchor left behind would nudge the next
    // visit off the first row. Clearing is a `Max-Age` of zero.
    const value = index <= 0 ? "" : `row=${code}&at=${index}&list=${list.dataset.list || ""}`;
    document.cookie = `${AT_COOKIE}=${value}; Path=/; Max-Age=${value ? AT_MAX_AGE : 0}; SameSite=Lax`;
  });

  // Restoring runs once, here, at parse time — this script is `defer`, so the document is built.
  // Deliberately not in a `pageshow` listener: a back/forward-cache restore already has the real
  // scroll position intact, and a second one of ours would fight the browser's. Nothing here touches
  // the browser's own history-scroll setting either, for the same reason — a back button is the one
  // case where it already knows better than we do.
  const restoreList = document.getElementById("rows");
  const wanted = restoreList && restoreList.dataset.anchor;
  if (wanted) {
    let row = null;
    for (const candidate of rowsIn(restoreList)) {
      if (candidate.dataset.row === wanted) {
        row = candidate;
        break;
      }
    }
    // One frame, so the sticky bar has settled. `scrollBy` over a measured delta rather than
    // `scrollIntoView`, which would want a `scroll-margin-top` and therefore a bar height nothing
    // declares.
    if (row) {
      requestAnimationFrame(() => {
        window.scrollBy(0, row.getBoundingClientRect().top - barBottom());
      });
    }
  }
})();
