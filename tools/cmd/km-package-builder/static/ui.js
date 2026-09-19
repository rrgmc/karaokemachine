// The tool's own JavaScript: four jobs, and none of them is a front end.
//
// **Saying out loud when a request failed** is the first and the oldest, and the rest of this comment
// is about it. The other three are at the bottom of the file and are each a dozen lines: ticking a
// whole page of rows from the box in the table's head, ticking the run between two boxes from a
// shift-click, and restoring the page number on the requests that leave the list meaning what it
// meant -- the two filter-bar controls that change neither which songs match nor how many, and the
// one action that sets no filter at all. All three are things htmx has no opinion about rather than
// gaps in it, and all three are deliberately the *only* state this file keeps -- the page is
// server-rendered and stays that way.
//
// **A selection is not among that state, and the run is the case that looks like it is.** What the
// shift-click remembers is which box the last press was on, which is the browser's own account of
// what just happened rather than a model of the page; the selection itself stays where it has always
// been, in the boxes, and is read off them by `hx-include`. Both die with the block they are in.
//
// htmx does not swap the response of a failed request -- a 4xx or 5xx body could be
// anything, and painting it into the page is worse than ignoring it. The consequence here is that
// `handlers::failure` returns a 500 with the reason in it and **nothing at all happens on screen**:
// the list dims for a moment, the button re-enables, and the page is exactly as it was. Turning a
// page over a database that answered with an error looks identical to turning a page that had no
// next page, which is the report this was written from.
//
// So the reason has to be put on screen by the only thing that saw it, which is the browser. **No
// status code changes**, and that is still deliberate rather than unfinished: a `/songs/rows`
// failure answered with a 200 and a bubble would swap that 200 into `#rows` and blank the table --
// the error would cost you the rows you were reading. The 500 keeps the rows where they are,
// and this file says why they did not change.
//
// **There is a server half now, and this comment used to say there never would be.** What was added
// is the opposite case, and it does not touch anything above: an action that *succeeded*, taken from
// a list. Those come back as a 200 with an out-of-band `#toasts` swap in them (`views::toast_only`),
// because the message slot they used to fill is above a page of rows and a person pressing a button
// at the bottom of the list never saw it. htmx does that insertion, so this file no longer fills the
// tray -- it *manages* it, which is the part that has to be in one place: a toast the server sent and
// a toast a failure produced fade the same way, dismiss the same way, are removed by the same three
// paths, and are counted against the same cap when a bulk action produces a burst of them. `arm` is
// where the first lives, `trim` the second, and the observer below is what reaches a server toast.
//
// The tool's own refusals on a *page* are a separate thing and are not touched here: those come back
// as a 200 with a `.message` in it, land in the slot beside the button that caused them, and stay
// there to be read.

(() => {
  "use strict";

  // Longer than km-remote-pages's four seconds, and the difference is the audience. That one says "Queued:
  // Tempo Perdido" to somebody holding a microphone; this one says what a database did, in a
  // sentence that has to be read rather than recognized.
  const LINGER_MS = 8000;

  // A refusal from SQLite can run to a paragraph, and the tail of one is rarely the useful part.
  // Cut rather than scroll: the whole message is in the tool's log, which is where a person who
  // wants all of it should be looking.
  const MAX_CHARS = 400;

  // How many may be on screen at once. Four is what fits under the header without the strip reading
  // as a second page, and a bulk action over a hundred rows can produce more than that in a burst --
  // at which point the older ones are already read or already stale, and burying the newest under
  // them is the failure worth avoiding. Each still leaves on its own timer; this only trims the tail.
  const MAX_TOASTS = 4;

  // **Every sentence this file says is read off the page, and none is written here.** A static file
  // cannot go through askama's `|t` filter, so an English string in this one would appear inside a
  // Portuguese page with nothing to catch it: the catalog scanner reads templates only, and the page
  // tests fetch pages and never see this script. `layout.html` puts them on `<body>` as `data-js-`
  // attributes, which is how `km-remote-pages`'s `scan.js` already reads its own.
  function words(name) {
    return document.body.getAttribute("data-js-" + name) || "";
  }

  // Two of those sentences carry values only the browser has -- which request failed, and how it
  // answered -- so the catalog composes them with `{what}` and `{status}` standing in, and this puts
  // the values where the sentence wants them. **Not a template engine, and it does not become one**:
  // nothing here builds markup, the result goes into `textContent`, and a sentence with no marker in
  // it comes back unchanged.
  function fill(text, values) {
    for (const key in values) text = text.split("{" + key + "}").join(values[key]);
    return text;
  }

  function tray() {
    return document.getElementById("toasts");
  }

  // The tray is newest-first in the DOM -- `prepend` here, `afterbegin` from the server -- so the
  // oldest is the last child and dropping from the end is dropping the oldest.
  function trim(into) {
    while (into.children.length > MAX_TOASTS) into.lastElementChild.remove();
  }

  // Gives a toast its three ways out, wherever it came from.
  //
  // Three, and the belt-and-braces is not superstition. `animationend` is the ordinary one; the
  // timer covers a background tab, where the animation may never run to completion; and the click is
  // for a message somebody wants gone before either.
  //
  // `data-armed` is what stops a toast being armed twice -- the observer below sees every insertion,
  // including the ones this file makes.
  function arm(item) {
    if (item.dataset.armed) return;
    item.dataset.armed = "1";
    const drop = () => item.remove();
    item.addEventListener("animationend", drop, { once: true });
    item.addEventListener("click", drop, { once: true });
    setTimeout(drop, LINGER_MS + 2000);
  }

  // `textContent`, never `innerHTML`. What arrives here is a server's response body, and on the
  // sendError path a browser's own wording -- neither is markup this page authored, and one of them
  // is a string a corpus could have influenced by way of a file name in an error message.
  //
  // (A toast the *server* rendered goes through askama's escaping instead, which is the same
  // guarantee by the other route, and is why it may be inserted as markup by htmx.)
  function toast(text, level) {
    const into = tray();
    if (!into) return;

    // Clicking *next* twice against a stopped server should say one thing, not two. Only the newest
    // is compared: two different failures still both appear, and the same one repeated after the
    // first has faded is worth saying again.
    const newest = into.firstElementChild;
    if (newest && newest.dataset.text === text) return;

    const item = document.createElement("div");
    item.className = "toast-item " + level;
    item.setAttribute("role", "status");
    item.dataset.text = text;
    item.textContent = text;

    arm(item);
    into.prepend(item);
  }

  // Arms a toast the server sent.
  //
  // A `MutationObserver` rather than an htmx event, and the reason is that this has to survive being
  // right: an out-of-band swap is announced by more than one event depending on how htmx got there,
  // and a toast that is never armed is a message that stays on the screen for ever with
  // `pointer-events: auto`, swallowing clicks meant for the page under it. Watching the tray itself
  // cannot miss an insertion, whatever put it there.
  //
  // The cap is applied from here for the same reason the arming is: it is the one place both sources
  // arrive at. Removing a child from inside the callback queues more records, but they carry only
  // `removedNodes` and a second `trim` finds nothing left to do, so this does not feed itself.
  function watchTray() {
    const into = tray();
    if (!into) return;
    for (const item of into.children) arm(item);
    trim(into);
    new MutationObserver((records) => {
      let arrived = false;
      for (const record of records) {
        for (const node of record.addedNodes) {
          if (node.nodeType === 1 && node.classList.contains("toast-item")) {
            arm(node);
            arrived = true;
          }
        }
      }
      trim(into);
      // The tray is about to be looked at, which makes this the moment its offset has to be right.
      if (arrived) measureHeader();
    }).observe(into, { childList: true });
  }

  // Keeps `--header-height` equal to the header the toast tray has to start below.
  //
  // The header is `position: sticky` at the top of the viewport and a wrapping flex row, so its
  // height is a function of the window width rather than a constant `style.css` could hold.
  function measureHeader() {
    const header = document.querySelector("header");
    if (!header) return;
    const height = Math.round(header.getBoundingClientRect().height);
    document.documentElement.style.setProperty("--header-height", `${height}px`);
  }

  // Two ways of staying current, for the reason `arm` has three.
  //
  // A `ResizeObserver` rather than a `resize` listener is the ordinary one: the nav wraps at a width
  // the stylesheet chooses, not one this file knows, and observing the element itself needs to know
  // neither. But it delivers during a rendering update, which a browser skips entirely while the tab
  // is not being painted -- so a window resized in a background tab leaves the offset stale. The
  // second is `measureHeader` from the tray observer below, which runs at the only moment a stale
  // offset could be seen: the instant a toast is inserted.
  function watchHeader() {
    measureHeader();
    const header = document.querySelector("header");
    if (!header) return;
    if (window.ResizeObserver) new ResizeObserver(measureHeader).observe(header);
    else window.addEventListener("resize", measureHeader);
  }

  function shorten(text) {
    const trimmed = (text || "").trim();
    if (!trimmed) return "";
    return trimmed.length > MAX_CHARS ? trimmed.slice(0, MAX_CHARS) + "…" : trimmed;
  }

  // What was asked for, so a message can name it. The path alone, because the query string on a
  // browse request is forty characters of filter state and says nothing a person needs here.
  function what(detail) {
    const path = detail && detail.pathInfo && detail.pathInfo.requestPath;
    return path ? path.split("?")[0] : words("the-tool");
  }

  // The server answered, and said no.
  document.body.addEventListener("htmx:responseError", (event) => {
    const xhr = event.detail.xhr;
    const said = shorten(xhr && xhr.responseText);
    const status = xhr ? xhr.status : 0;
    // The body is the whole message when there is one -- `failure()` puts the error itself there,
    // and a person reading this tool's screen is a person who can use it. The status is the
    // fallback for a response that carried no body, where a bare "that failed" would be useless.
    const said_or_status =
      said || fill(words("answered"), { what: what(event.detail), status });
    toast(said_or_status, status === 404 ? "toast-warn" : "toast-bad");
  });

  // Nothing answered at all. Far and away the likeliest cause here is that the tool was quit -- from
  // its own Quit button, from the console it was started in, or by closing the window -- leaving a
  // page that still looks alive on screen. So the message says that rather than talking about the
  // network, which for a server on loopback is not what went wrong.
  document.body.addEventListener("htmx:sendError", () => {
    toast(words("unreachable"), "toast-bad");
  });

  document.body.addEventListener("htmx:timeout", (event) => {
    toast(fill(words("timed-out"), { what: what(event.detail) }), "toast-warn");
  });

  // The reply arrived and could not be put on the page. That is this tool's own bug rather than
  // anything the person did, and saying so is better than a screen that silently did not update.
  document.body.addEventListener("htmx:swapError", (event) => {
    toast(fill(words("swap-failed"), { what: what(event.detail) }), "toast-bad");
  });

  // Clicking a discovered machine puts its address in the box beside the Save button.
  //
  // The *saving* is htmx's -- the button carries `hx-post="/settings"` and `hx-vals` -- and this only
  // keeps the visible field honest, which htmx cannot do because the field is not what it swapped.
  // Without it the message says the machine was saved while the box above still shows the old
  // address, which reads like the click did not work.
  //
  // Delegated, and reading the address from a `data-` attribute rather than having the template
  // write it into an `onclick`: the value then never passes through a JavaScript string literal, so
  // there is no quoting to get right in a template that is otherwise plain HTML.
  document.body.addEventListener("click", (event) => {
    const button = event.target.closest("[data-machine]");
    if (!button) return;
    const field = document.querySelector("input[name=machine]");
    if (field) field.value = button.dataset.machine;
  });

  // -- ticking a whole page ------------------------------------------------------------------
  //
  // The box in a song table's first header cell: the Songs page's `#rows` and the similar-names
  // page's `#hits`. It has no `name` and is never submitted; all it does is set the boxes that are,
  // which is what keeps each page's `hx-include` unchanged.
  //
  // **The table is the scope, not a page's id**, so one box never reaches the other list's rows and
  // a page with a new song table needs nothing here.
  //
  // Delegated, because the table is replaced outright by every filter change and every page turn, so
  // anything bound to the box itself would be bound to an element that is about to be thrown away.

  function rowBoxes(table) {
    return table.querySelectorAll("input[name='song_id']");
  }

  // Keeps the header box honest about the rows under it: ticked when all are, indeterminate when
  // some are. Without this, unticking one row after *select all* leaves a box claiming the page is
  // fully selected, and the next click on it *unticks* rather than completing the selection.
  function syncSelectAll(row) {
    const table = row.closest("table");
    const box = table && table.querySelector(".select-all");
    if (!box) return;
    const rows = rowBoxes(table);
    const ticked = Array.prototype.filter.call(rows, (r) => r.checked).length;
    box.checked = rows.length > 0 && ticked === rows.length;
    box.indeterminate = ticked > 0 && ticked < rows.length;
  }

  document.body.addEventListener("change", (event) => {
    const target = event.target;
    if (target.classList && target.classList.contains("select-all")) {
      const table = target.closest("table");
      if (table) for (const row of rowBoxes(table)) row.checked = target.checked;
      target.indeterminate = false;
      return;
    }
    if (target.name === "song_id") syncSelectAll(target);
  });

  // -- ticking a run of rows -----------------------------------------------------------------
  //
  // Between one row and the whole page there is a run, and a person who has ticked a box and wants
  // the eleven under it reaches for shift. The gesture is the one a file manager and a mail client
  // both already answer, so the tool answering it with a single tick is the tool being wrong.
  //
  // **The run takes the state of the box that was shift-clicked**, which is what lets one gesture
  // clear a run as readily as tick one. A run that always ticked would need a second gesture to undo
  // a mis-aimed one, and the second gesture is the thing being got rid of.
  //
  // **The anchor is an element rather than a position, and that is what clears it.** `#rows` is
  // thrown away by every page turn and every filter change, and a row's own `hx-target="closest
  // tbody"` throws away one, so a box that has left the document says so itself. An index into the
  // list would survive all three and address whatever row had moved into its place.

  // The box last set by a plain click. It stays where it is while shift-clicks move the other end,
  // so a run is grown and shrunk from one place rather than re-anchored on every press.
  let runAnchor = null;

  // The boxes from one end to the other, or nothing when the two ends are not in one table. The
  // table is the bound because the table is what is drawn: a run cannot reach a row that is not on
  // the screen, which is the same page-shaped scope the box in the head means. The filter is not in
  // this and never is -- *every song matching* is a scope an action asks for in words, and it counts
  // and confirms before it writes.
  function runBetween(from, to) {
    const table = from.closest("table");
    if (!table || table !== to.closest("table")) return null;
    const all = Array.prototype.slice.call(
      table.querySelectorAll("input[name='song_id']"),
    );
    const first = all.indexOf(from);
    const last = all.indexOf(to);
    if (first === -1 || last === -1) return null;
    return all.slice(Math.min(first, last), Math.max(first, last) + 1);
  }

  // Shift with a mouse button is also how a browser is asked to select text, and without this the
  // run is ticked with every row between its ends painted in the selection colour. A checkbox's own
  // toggle is on `click`, so refusing the `mousedown` costs none of it -- it costs the focus ring,
  // which is the cheaper of the two and the only one of them a keyboard cannot get back.
  document.body.addEventListener("mousedown", (event) => {
    if (event.shiftKey && event.target.name === "song_id") event.preventDefault();
  });

  // `click` rather than `change`, and there is no choice in it: a `change` is a plain `Event` and
  // carries no `shiftKey`, so the modifier is legible on the mouse event alone.
  document.body.addEventListener("click", (event) => {
    const box = event.target;
    if (box.name !== "song_id") return;
    const run =
      event.shiftKey && runAnchor && runAnchor.isConnected
        ? runBetween(runAnchor, box)
        : null;
    if (!run) {
      // A shift-click with nothing to reach back to is a plain one, and sets the anchor it wanted.
      runAnchor = box;
      return;
    }
    // The box pressed has toggled by the time this runs, and the run follows it.
    for (const other of run) other.checked = box.checked;
    // Setting `checked` fires nothing, so without this the box in the head goes on describing the
    // selection as it stood before the run.
    syncSelectAll(box);
  });

  // -- which page a view change lands on -----------------------------------------------------
  //
  // Changing a filter starts again at the top, and that is right: it changes *which* songs match, so
  // an offset into the old set points nowhere in the new one -- at best somewhere arbitrary, at worst
  // past the end, which is an empty list with a working "previous" button.
  //
  // Two controls in that bar change nothing about which songs match. `sort` reorders them and
  // `filename` only decides whether a chip is drawn, so page four still means a real page four.
  //
  // *Title from file name* keeps its page for a plainer reason: it touches no filter at all. It
  // writes the ticked rows and redraws the list they are in, so the page being read is still the page
  // being read, and landing back at the top throws away where somebody had got to in a corpus.
  //
  // The whole mechanism is here rather than in a handler: the server already writes where it is onto
  // `#rows` (`data-offset`/`data-total`), so this is a read and a copy, with no route, no form field
  // and nothing for `rebuild` to keep in step. A form field is also the one shape that cannot work --
  // `#rows` rides in the body beside `#filters`, and a second `offset` in one body is a 400 that
  // takes a button out of service with nothing said anywhere.

  // The controls whose change leaves the page number meaning what it meant.
  const KEEPS_THE_PAGE = ["sort", "filename", "warnings"];

  // A form that writes rather than filters keeps its page however it was pressed, and says so in its
  // own markup with `data-keeps-the-page`. An attribute rather than a list of ids here: a third such
  // action is then one template change, and cannot be added without the page it is added to saying
  // what it wants.

  // What was last changed inside the filter bar, for the request it is about to trigger.
  let lastFilterChange = null;

  // Copies where the list is onto the request about to be made.
  //
  // `withTotal` is for the two bar controls, which change neither which songs match nor how many: the
  // count this page was rendered with is still the right one and need not be taken again. A retitle
  // rewrites a title and empties an artist, so a list narrowed by the search box, by an artist or by
  // an initial holds fewer songs afterwards and the carried count would be a number that is not true.
  // Absent, it is counted again.
  function putThePageBack(event, withTotal) {
    const rows = document.getElementById("rows");
    if (!rows) return;
    if (rows.dataset.offset) event.detail.parameters.offset = rows.dataset.offset;
    if (withTotal && rows.dataset.total) event.detail.parameters.total = rows.dataset.total;
  }

  // **Capture phase, and that is load-bearing.** htmx's own `change` handler is on the form, which is
  // below `body` on the way down and above it on the way up -- so a bubbling listener here would run
  // after the request had already been configured and would be reading the *previous* change.
  document.body.addEventListener(
    "change",
    (event) => {
      const bar = event.target.closest && event.target.closest("#filters");
      lastFilterChange = bar ? event.target.name : null;
    },
    true,
  );

  document.body.addEventListener("htmx:configRequest", (event) => {
    const elt = event.detail.elt;
    if (!elt) return;
    // The offset alone, and without asking what was last changed: such a form has one button and the
    // press is the whole of what it is answering.
    if (elt.dataset && "keepsThePage" in elt.dataset) return putThePageBack(event, false);
    if (elt.id !== "filters") return;
    // Read once and clear, so that a request the bar makes for another reason cannot inherit it. The
    // case that matters is typing in the search box: that fires `keyup`, never `change`, and a stale
    // `sort` here would hold the reader on page four of a list they have just narrowed.
    const changed = lastFilterChange;
    lastFilterChange = null;
    if (KEEPS_THE_PAGE.indexOf(changed) === -1) return;
    putThePageBack(event, true);
  });

  watchTray();
  watchHeader();
})();
