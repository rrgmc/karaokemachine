# The remotes

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## Dev remote layout

**One column on a narrow screen, and it is still not a singer's remote.** It carries the debugging
switch, `debug/play-file` and a raw event log, none of which belongs on a phone at a party — so
`km-remote-pages` at `/` is the answer to that and this is not competing with it.

**This row said desktop-only, and told anybody tempted to add a media query to change it first.**
The argument was that making the page *fit* a phone would still not make it *right* for one, which
is true and answers a question nobody was asking. The person carrying this page around is usually
carrying it *to* the machine — to press Play on something, or to watch the event log while a box
under a television restarts — and a layout that could not be read on the way there was costing
something for nothing.

**What it took was two lines rather than a redesign, which is most of why the old row was wrong
about the price.** `auto-fit` with `minmax(20rem, 1fr)` drops columns until one is left and then
keeps that one at its 320px *minimum*, inside a 328px content box on a 360px phone — so any table a
few pixels wider scrolled the whole page sideways. `min(20rem, 100%)` lets the last track shrink to
the page. A `max-width: 34rem` query gives back the body padding, which was a fifth of the width, and
grows the buttons. **Nothing is reordered and nothing is hidden**: the groups, the order and every
control are what they are on a desktop.

**Grouped by concern, in the order somebody uses one**: Session, Playing, Catalog, The machine,
Diagnostics. **Diagnostics last is the half that is load-bearing** — the debugging switch,
`debug/play-file`, the raw event log and the machine's own log are what this page has and a product
surface does not, so they belong where somebody scrolls to them deliberately.

**The machine's log is a second pane and a second socket, beside the event transcript rather than
inside it.** That transcript is a record of what a *client* does, and the machine's own lines folded
into it would leave neither legible. The two sockets keep their own reconnection state for the same
kind of reason — one stream coming back must not reset the other's wait.

**On-page prose is one line per control, and the reasoning lives in the comment beside it.** That is
[`tools/CLAUDE.md`](../../tools/CLAUDE.md)'s rule for every surface under `tools/`, and this page had
drifted a long way from it — three paragraphs explaining what a route name says, and several of them
naming ACL route ids (`audio.write`, `demo.write`, `debug.play_file`) that stopped existing when the
prefix became the permission. A developer reading this page does not need a paragraph to be told a
409 means something is playing.

## Dev remote in release builds

**Compiled into the binary so no build can ship without it, and served only when somebody asks** —
`--dev-remote` for one run, `api.serve_dev_remote: true` **and `debug.enabled: true`** for good. Off
is the default in every build, and in `ApiConfig::default()` too, so the layer that reads settings
and the layer that does not agree. One of them following `debug_assertions` while the other answers
`true` is how they come apart.

**Both switches are required**, because the console's own API asks for no password —
[`The development console has an API that needs no password`](api-and-network.md#the-development-console-has-an-api-that-needs-no-password)
is the argument. `--dev-remote` turns on both, and says so in `--help`: there is no `--debug` flag to
pair it with, so a flag setting only its own half would silently do nothing on the machines it exists
for.

**Both admin surfaces carry the switch, beside debugging on the same pane.** The trap a separate pane
would set is that the console does nothing without debugging, so somebody would turn one on and see
no change with nothing to explain it.

**On by default would be the wrong answer.** The argument for it is that it withholds a page rather
than a permission — every route it drives is one the machine already governs to the same caller —
but that answers a question nobody is asking: the objection to a developer's console reachable by
default on every machine on the LAN is not that it escalates, it is that a karaoke machine should
not publish one. Not shipping a console is the point, and `/` is the singer's remote and `/admin/`
the owner's page, so nothing an owner needs is behind this.

**The flag exists as well as the settings key** because a curator whose package builder is refused
an upload has a flag to type instead of a file to find on a machine in another room, and
`km-package-builder`'s refusal names both. It is one-way, turning the console on for a run and never
off, because a machine told in settings to serve it should not be silenced by a run that did not
think to mention it.

## The dev remote comes back on its own

**The `/dev/` page reconnects its event stream by itself, and re-reads everything when it succeeds.**
This is the one page whose purpose is to be open while the machine it points at is restarted, and the
rebuild-and-restart cycle is what a development remote is *for*.

**A second doubling to thirty, unbounded**, which is the offline remote's policy: coming back to an
address that is coming back is cheap and worth trying often, a machine left off should not be asked
every second all afternoon, and there is no attempt count at which giving up and making somebody find
a button becomes the better answer. **The drop is logged once rather than once an attempt** — a
machine off for an hour would otherwise push every useful line out of a log whose job is to be a
readable record of what a client does.

**A reconnection re-reads everything, because a restarted machine agrees with the page about
nothing**: the queue is empty and the settings came off disk again. The refresh happens on the reopen
rather than on the attempt, and the log says `websocket reopened` rather than `websocket open`, since
a transcript that cannot tell the two apart is missing the interesting half. **The button stays**,
because it is also how the page is pointed at a different machine and it means *refresh everything*
besides. Pressing it resets the backoff: somebody pressing it is saying they think the machine is
back, which is a better guess than the interval a run of failures had arrived at.

**A restart does not take the admin token with it.** A token held in memory and forgotten across a
restart leaves a reconnection that mends only the socket looking fixed while every admin widget
refuses. A token is an HMAC over the stored password hash, verified by recomputation, so it survives
the restart and the page comes back working — see
[`A machine gives itself a password, and shows it on the television`](api-and-network.md#a-machine-gives-itself-a-password-and-shows-it-on-the-television).

**The recovery it needed is kept, because the token can still stop working**: the password changed,
or somebody signed out everywhere, or twelve hours passed. A 401 against a request that carried a
bearer clears the token, empties the field as a logout would, and writes why into the admin note.
**`/admin/login` is excluded from that, and the exclusion is load-bearing rather than tidy**: that
route sends the bearer like every other, so a mistyped password would otherwise throw a live token
away.

**The notice arrives on the first admin *action*, not at the moment the socket returns**, and that is
a property of the route surface rather than a choice: every `GET` on this surface is outside
`/api/v1/admin/`, so there is no admin-gated read to test a token against, and testing one with a
write would mean changing the machine to ask a question about it.

## Web remote

**Built, and rendered by the machine rather than dropped in as files.**
`crates/remote/km-remote-pages` is one set of askama templates and htmx handlers serving two modes,
linked into `km-app` for the online one and into `km-remote` for the offline one. There is no
`ServeDir` seam beside it: two mechanisms that can disagree about what `/` is are what the
`Where a song's media lives` and `Where packages live` rows already refused twice.
`api.serve_remote: false` turns it off, matching `serve_dev_remote`.

## Two remotes, and which is the smaller one

**The offline app is the whole remote; the one the machine serves is a subset of it.** Favorites with
folders and the A–Z filter belong to `km-remote` and are **absent** from the machine's own remote
rather than present and disabled. Two reasons, and they are different. A favorites collection living
on the appliance would be a shared list nobody owns, edited by every phone in the room with no story
for two of them at once — a personal list belongs on the phone that holds it. And the A–Z filter
needs an indexed folded-initial column that the mirror carries and `library.sqlite` does not, so it is
not a thing the machine is withholding. Both catalogs carry folded sort keys per
`One alphabet, everywhere` in [`songs.md`](songs.md), so the ordering is the same on both sides and
only the folded *initial* is the mirror's alone.

**Absent and not disabled is the rule for a whole *mode*.** A control the loaded **song** refuses —
transpose on a video — is drawn and grayed instead, because the machine states that per song
precisely so a client can.

**This row says which two features are the offline app's, and it should not be read as saying that
the offline app is where features land.** A feature belongs to both unless one of those two reasons
bites: a collection somebody owns, or a column the mirror has and `library.sqlite` lacks.

## The remote's palette

**Light, and pinned light — no dark palette and no `prefers-color-scheme` branch.** A phone set to
dark mode still gets the light remote, matching the owner's Go remote for a Videoke unit
(`cookara-re`), which is the working reference for this product in this room. **The television is
unaffected**: `km-display`'s theme stays dark, because a screen across the room and a screen in a hand
are not the same problem, and the remote keeps that theme's *hues* at the contrast a white ground
needs.

## The A–Z filter is a combobox

**One `<select>` in the filter row, not twenty-seven links.** The remote's link/swap rule says
anything changing the *bar* is an ordinary page load and anything narrowing the *list* is an htmx
swap. A strip of letters draws "which letter is current" a second time, as a highlight, so a swap
leaving the bar alone could put that highlight out of step with the rows under it. A `<select>` has no
second drawing: its value **is** the highlight, the browser owns it, and the letter joins the search
box and the language picker in the form, so the three compose — pick S, then type, and you are
searching inside S. It costs a page load per letter and buys a control that fits one line beside the
language picker instead of two rows of buttons above it, rendered by a phone as its own native list.
Matching `cookara-re`, which reached the same answers first.

**The digit bucket's value is `#` and its label is `0–9`.** The value is what `BrowseParams::initial`
parses and what `prefs::browse_state` writes into the `km_browse` cookie; the label is the only half
that can move. `#` beside twenty-six letters is a hash character, while `0–9` is a bucket.

**The parameter is `?initial=`**, matching `km_song::text::initial`, the function every surface
already calls to produce the value, and the package builder's own filter. See
`One spelling per concept, across every surface` in [`foundations.md`](foundations.md).

**The empty option says `All`.** It clears the filter, and what comes back is every song in the
catalog — the `0–9` ones among them, so a control naming a range that excludes the bucket directly
above it would not be describing what it does. `All letters` is the same claim one word longer, and
length is not free for a `<select>`: it is as wide as its widest option, and the row already carries
the language picker and the ⋯ toggle. That the language picker says `All` too is not a collision —
each drops down to what it filters, so the word is the same because the act is.

## The tag filter is an add-select and a row of chips

**A `<select>` that adds one tag, and the chosen ones drawn beside it with an ✕ each.** The two other
filters on that row show what is chosen by *being set to it* — a picker's value is its own highlight,
which is the whole argument the A–Z combobox above rests on. A tag filter holds a *set*, so it cannot:
whatever the picker last added, it has to go back to offering the rest. So the state is drawn, and
the control that changes it does one thing.

**Not a `<select multiple>`**, which renders as a scrolling list box on iOS and is unusable with one
thumb, and needs JavaScript to join its values into the one comma-joined parameter the wire takes.
Not a `<details>` of checkboxes either: compact when closed, but the chosen set is invisible until it
is opened, which is the state a filter most needs to show.

**The picker offers only what is not already chosen, and is not drawn when nothing is left.** An
option that changes nothing is not a choice. A catalog nobody has tagged therefore draws no tag
control at all — which is most catalogs, because nothing detects a tag, so this is the ordinary case
rather than an edge one.

**Each chip's ✕ is an htmx swap and not a plain link**, which is the package builder's chips read the
other way round. The rule is the browse block's own: a control that only refilters the list swaps it,
and only a change of *which* list you are looking at is a full page load. Taking a tag off changes
the chip strip as well as the rows, so it asks for the whole `#browse` fragment, exactly as the ✕
beside the search box does. The remaining tags travel in that link's own URL rather than through
`hx-include`, because the hidden field still holds the old set at the moment it is pressed — where
the language and the initial do the opposite, being live `<select>` values.

**Two names for what looks like one control**, and it is a 400 that forces it: the chosen set rides
the form as a hidden `tags` field, so a picker also called `tags` would put that key in the request
twice. `serde_urlencoded` refuses a repeated known key, and htmx does not swap on an error — so the
bar would stop responding with nothing said anywhere. The picker is `add_tag`, and the handler merges
it in and renders with it cleared.

**Nothing about a song row changes.** Tags narrow the list and are never drawn beside a song: a phone
is narrow, and a run of chips on every row would cost more width than a filter is worth. The curation
tool draws them, because that is where curation is checked.

## Starting a song immediately

**`▶ now` is behind the ⋯ toggle, and it is the elevated half of what a row offers.**
It is queue, move to the front, and then end what is playing so the front is reached — three calls
where `↑ next` is two, because the API has no "play this one instead" and giving it one would put a
remote's furniture inside the machine's queue.

**Neither half is an admin route, so the judgement is all that governs it**: adding is what this
machine offers the whole room, and interrupting is not. **So it is off by default** — one guest
should not be handed a one-tap way to end another guest's song beside every search result. Two taps,
remembered per phone.

## The transport is folded away, and only on the Queue tab

**Pause, restart, skip and stop are not on screen until the ▾ beside the song's name is pressed, and
the Now tab does not carry them at all.** Both remotes.

**The mark is a chevron that flips — ▾ closed, ▴ open — and not the ⋯ the queue rows below it
carry.** The two ask different questions. **⋯ on a row means *there is more you can do to this*, and
it opens actions that were not on the screen at all; this one means *this panel has a second half*.**
The panel is right there and its lower half is folded; pressing it unfolds the same thing rather than
revealing anything new, and a chevron is the mark for that.

**A chevron also says which way it is folded, and ⋯ cannot.** This is the only control on the tab
with an open state to report, and a mark identical in both states leaves the amber border to do that
whole job — a color difference, on a phone, in a dark room.

**One vocabulary is not a defense of two identical buttons on one screen.** The queue rows' ⋯ sits
directly below this, and three dots twice with nothing to tell them apart reads as the same button
drawn twice rather than as a shared idiom.

**The gear is the [Setup tab](#setup-is-a-fourth-tab-and-it-is-the-narrow-one)'s** and is ruled out
here: one glyph meaning two things in one application is worth less than either meaning.

**Both glyphs are in the markup and the stylesheet hides one**, so the flip costs no JavaScript —
the property this whole disclosure is built on, and the thing a reimplementation loses first. The
label's `aria-label` deliberately does *not* flip with it: a `<label>` cannot carry a truthful
`aria-expanded` without script, and *Show controls* naming what the press is for beats a label that
changes under a screen reader while the checkbox it fronts says nothing.

**The four of them end somebody's turn, and this is a phone in a dark room.** Every other control in
this remote adjusts something: the key, the tempo, the volume and the guide melody all change a
performance in progress and are undone by pressing them back, and queueing a song is undone by taking
it out again. Skip is not undoable — the song is gone, the next one is loaded, and the person singing
has lost their turn in front of a room. A control whose worst case is that and whose target is a
2.1rem square under somebody's thumb is a fault waiting to happen rather than a convenience.

**On the Queue tab and not the Now tab.** Somebody reading the queue is looking at whose turn is next,
which is the context in which taking the current turn away is a considered act; somebody on the Now
tab is watching a song play. **The Now tab keeps the four control rows**, and the line between them
and the four that are not there is the whole of this decision: those adjust a performance, and the
transport ends one.

**A disclosure and not a confirm dialog.** A confirmation would put a second tap in front of every
skip for ever, including the twenty deliberate ones an evening has; a disclosure puts it in front of
the first and then gets out of the way.

**It closes when you leave the tab, and it does not close when the song changes.** Two separate
properties, each ruling out an obvious implementation. The ⋯ toggle is remembered per phone in a
cookie because the question it asks is whether this phone does that sort of thing at all; this one
asks whether you meant *this*, so an answer that outlived the visit would put the buttons permanently
back on screen. And the now bar is republished by the pump whenever the song or the settings change,
so an open strip must survive a swap or it would fold away mid-song.

**Both fall out of a hidden checkbox in the page and a `<label>` in the fragment**, which is why there
is no JavaScript here at all. The state is outside the element the pump replaces, so a republish
cannot touch it; and every tab in this remote is an ordinary link, so arriving at the Queue tab
renders the checkbox afresh and unchecked.

**What is playing sits at the top of that tab, with no progress bar.** Somebody looking at whose turn
is next is exactly the person who wants to pause, restart, skip or stop what is on. It is also the
page where the omission is least visible, because the playing song is genuinely *absent* from the
list — `km_queue::Queue::pop` takes it off the front when it starts. **The progress bar is
deliberately not repeated**: the position is the only part of the card that moves, it goes out as its
own fragment once a second, and a second copy would be a second fragment republished every second for
something nobody reading a queue is watching.

**It shows what is playing as well as what is waiting.** The page already shows every waiting song's
title, artist and singer, and what is playing is the head of the queue as a singer thinks of it. The
tab is one view of the queue, including its head.

## A queue row's controls are behind a ⋯ too, and it no longer asks

**`↑ ↓ ✕` are folded away until the ⋯ above the list is pressed, and the ✕ carries no `hx-confirm`.**
Both remotes.

**The disclosure's reason is this list's rather than the transport's or the Songs tab's.** These are 2.1rem
squares laid along the right edge of *every row* in a list somebody scrolls, they reorder and remove
songs other people are waiting on, and a queue is read far more often than it is edited. A thumb
traveling down the page must not be able to take somebody's turn away by landing slightly wrong.
**Its lifetime is the page's**, like the transport's and unlike the Songs tab's ⋯: that one asks whether this
phone cuts ahead of people at all, an answer given once for an evening, and this asks whether you
meant to edit the queue *now*. Both are a hidden checkbox outside `#queue` and a `<label>` inside it,
because the pump republishes that fragment whenever anybody queues anything and a state held in there
would fold the buttons away under somebody's finger.

**`hx-confirm` is not usable in this product at all.** It calls `window.confirm()`, and a `WebView`
with no `WebChromeClient` or a `WKWebView` with no `WKUIDelegate` answers `false` with nothing on
screen — so htmx reads "the person said no" and never sends the request. **Both mobile applications
answer the page's dialogs**, which is what keeps the Songs tab's folder ✕ working. On top of that, a
confirm behind a ⋯ is the second tap the transport decision already argued against, and taking your own
song out of a queue is undone by queueing it again.

**The folder ✕ still asks, and that is not an inconsistency.** Nothing is folded in front of it, it
edits a list somebody built up over weeks rather than an evening, and putting a song back into a
folder means finding it again.

## Where the extra row actions live

**A ⋯ toggle in the filter row that reveals `↑ next` and `▶ now` on every row at once, remembered per
phone — not a menu per row.** The alternative was a per-row overflow opening the bottom sheet the star
already uses, which would cost no new markup and keep every row to two buttons. It was declined
because the question these controls raise is not *which song* but *whether this phone does this sort
of thing at all*: somebody who wants to cut ahead wants it for the evening, and somebody who does not
should never meet the control. One answer, given once, is the shape of that.

The toggle is a parameter on the browse route rather than a route of its own, because a route of its
own would be a route on this page that reaches no machine at all — and every other one does.

**The ✕ that takes a song out of a favorites folder is not behind it**: it edits this phone's own
list, it asks before it acts, and it reorders nobody's evening.

## The ⋯ toggle draws the rows; it does not fetch them

**The two buttons are in every row from the start and `app.css` decides whether they are visible, so
pressing ⋯ costs no request at all and the one request it does make draws nothing.**

**Which rows there are is not what this control changes.** The link/swap rule at the top of
`_browse.html` sorts controls by whether they change the bar or narrow the list, and this one does
neither — it changes how a row draws itself. So it belongs in a third group, where the browser answers
the press and the server is told afterwards.

**Re-rendering `#browse` here is the trap to avoid.** `Load more` appends pages into the DOM and
nothing else knows they are there, so a re-render comes back as page one and throws away everything
read past. The visible symptom is not the missing rows: it is the **scroll jumping to the end of the
list**, because the document abruptly becomes a fifth of its height and the browser clamps a scroll
position past the new bottom.

**Hiding the two buttons changes no row's height**, because the queue `+` beside them is drawn
whatever this box says and an `.icon-btn`'s `min-height` is what holds the line. So the press moves
nothing under a reader, and
[`The list carries on by being scrolled to`](#the-list-carries-on-by-being-scrolled-to) is not
reachable from here: a control that shortened the document would clamp the scroll at the bottom of a
list, and a clamp is a `scroll` event like any other.

**A hidden checkbox and a `<label>`, which is `The transport is folded away` one page over**, and
the two differ in exactly one property: lifetime. The transport asks whether you meant *this* skip, so
arriving at the tab renders it afresh. This asks whether the phone cuts ahead of people at all — an
answer given once for an evening — so the box is checked from the `km_extra` cookie on arrival and a
press writes it back. The write is the only request, it is answered `204`, and **the checkbox carries
the value rather than the URL carrying it**: htmx sends a ticked box's `actions=1` and omits an
unticked one, so the server reads the state that now holds. Baking `actions=0`/`actions=1` into the
button's own URL is correct only for as long as something re-renders that button, and this control
deliberately does not.

**Markup a guest can un-hide buys them nothing, and it never did.** The toggle was never what stood
between a phone and those routes — the machine's own check was, and both routes are open to the whole
room by design. What the toggle decides is what an owner's guest is *offered*, which is a question
about the room rather than about the server, and CSS answers exactly that question.

## The list carries on by being scrolled to

**Reaching the bottom of a list fetches the next fifty rows, and the `Load more` button stays exactly
where it is.** Every other way of narrowing this list answers a gesture: the search box, the A–Z
filter, the tag chips. The end of a list is where a thumb is moving fastest, and a control there is
the one thing in the tab it has to stop and aim at.

**The trigger goes on the button rather than replacing it.** htmx binds `revealed` to the element
that already carries the URL, so the press and the scroll ask for the same page by the same route,
and a browser that fires neither still shows a control that works. That polarity is the argument: an
enhancement on top of a button degrades to the button, where a sentinel that replaces the button
degrades to a list that stops at fifty rows with nothing to press.

**`revealed` and not `intersect`.** htmx constructs an `IntersectionObserver` before it binds
anything and nothing in its trigger loop catches a throw, so a browser without that constructor loses
the whole `htmx.process` of the swapped-in page: the stars, the queue buttons and the action targets,
not merely this one control. A remote ships as an Android WebView shell, which is exactly where a
missing constructor cannot be investigated. `revealed` binds its listeners first and asks only for
`scroll`, `resize` and a timer.

**It cannot run away, and the bound is one page.** `revealed` fires as soon as it is bound if the
element is already on screen, so a chain needs a page of rows shorter than the viewport, and every
step adds a whole page to the document. Measured at 72 pixels a song row and 41 an artist row, a page
is 3600 pixels or 2050 against a column pinned at 26rem, so the depth is one at any viewport somebody
holds and two on a four-thousand-pixel screen turned portrait. Holding it to one with htmx's bracket
syntax costs the `Function` constructor these pages otherwise never reach, which is the worse trade.

**A dropped fetch is why the button has to stay visible.** `hx-sync="body:queue last"` allows the
document one request in flight and replaces whatever is queued behind it, discarding the loser with
no event to hear; htmx has stamped the button as revealed by then, so it never fires again. The list
goes quiet and a press starts it moving. Nothing detects that, and nothing needs to, because the
control that recovers it is the control that was already there.

**What it costs is the restore cap.** Five hundred loaded rows is ten flicks away, and past that cap
[`Coming back to the Songs tab comes back to the row, not to the top`](#coming-back-to-the-songs-tab-comes-back-to-the-row-not-to-the-top)
lands on the top of the list. Ten pages stands, because the number that replaces it has to come from
a measurement of how deep an evening goes rather than from a guess about it.

## Where you were lasts two hours

**`km_browse` is `Max-Age=7200`, and every Songs-tab request rewrites it**, so the two hours run from
the last time somebody was browsing rather than from the first. A phone put down between two songs
picks up the language, the tags and the search it had. A phone opened the next evening draws the top
of the catalog with nothing narrowing it.

**A remote that opens onto a filter nobody remembers setting reads as an empty catalog.** The filter
row is at the top of the list and the count beside it agrees with itself, so nothing on the page says
*these are not all the songs*: a singer sees a corpus of four hundred and concludes the machine has
lost the rest. That is the argument for hours rather than days, and it is the whole of it.

**One lifetime for the whole state.** The mode, the query, the language, the tags, the letter and the
artist or folder ride one cookie because they are one answer to *where was I*. Expiring the search
while keeping the language would land a phone on the plain song list with one narrowing still on it,
which is the confusing case arrived at deliberately.

**The browser is what expires it, so nothing stores a time.** `Max-Age` is exactly the question being
asked, *was this written recently enough*, and a stamp inside the cookie would want a clock on the
server, a comparison on every read, and a rule for a stamp dated in the future.

**A URL that carries a filter is not bound by this.** The filter form pushes its state with
`hx-push-url`, so a reload, a bookmark or a link shared between two phones restores that filter
however old it is. A URL that says something is itself the answer, which is the same rule that stops
the cookie overriding a link, and somebody who kept an address has not forgotten it.

## Coming back to the Songs tab comes back to the row, not to the top

**`km_browse` restores *which list* — the mode, the search, the language, the artist you had opened —
and a cookie of its own restores which row.** Somebody two hundred rows into a search coming back to
row one with the search intact is the half of "where you were" that reads as a fault rather than as a
limit.

**The tab bar stays plain links, and that is the constraint this is built around.** Boosting it would
keep the scroll for free, and it would also reopen
[`The transport is folded away, and only on the Queue tab`](#the-transport-is-folded-away-and-only-on-the-queue-tab)
and [`A queue row's controls are behind a ⋯ too, and it no longer asks`](#a-queue-rows-controls-are-behind-a--too-and-it-no-longer-asks),
both of which depend on a tab press *replacing the document* so that a disclosure cannot survive one.
So the position is reconstructed rather than preserved.

**An anchor and never a pixel offset.** Three reasons, and only the third decides it. Rows are not a
fixed height — `.song-name` wraps rather than truncating, so a rotation re-wraps a different set of
them and every saved pixel means a different song. The furniture above the list is not a fixed height
*between the two documents* either: `.browse-bar` is sticky with no declared height and grows with a
banner, an artist's heading and the filter row. And **a pixel offset cannot fail safely**: one that
misses still lands somewhere, and against a document that came back shorter the browser clamps it to
the end — the symptom recorded in
[`The ⋯ toggle draws the rows; it does not fetch them`](#the--toggle-draws-the-rows-it-does-not-fetch-them).
A row that is not found scrolls nothing, and nothing is the top of the list.

**Which row is "the one you were reading" has to be the row the restore re-establishes, or the list
walks.** The obvious spelling is the first row *overlapping* the bar's bottom edge, and it drifts:
measured against the real page, a row can clear that edge by **0.45 of a pixel** — `bottom` 137.98
against a bar bottom of 137.53 — and be, for every purpose a person has, entirely behind the bar.
Restoring that row puts it flush *below* the bar, moves the whole list down by one row's height, and
the next capture takes the row above it; three visits walk three rows up the list. The rule is instead
the first row whose **top** has reached that edge, which is a fixed point of the restore: align a
row's top to the line and the same rule selects the same row. Verified in a real browser — three round
trips from row 1138, `scrollY` 9914 every time, to the pixel.

**A cookie, and the reason is not idiom.** It is that the server has to know before it renders. Rows
past the first fifty exist only because `Load more` appended them and nothing server-side records
that, so an anchor kept in browser storage would mean the script pressing that button three more times
— three round trips, three swaps, each changing the scroll height under a scroll being set. In a
cookie, the tab press the browser was making anyway comes back with the right window in it.
`desktop.rs`'s claim that these pages use no `localStorage` therefore stands, and whoever moves this
to browser storage owes that doc comment an edit in the same change.

**A third lifetime, an hour.** `km_browse` is where you were this evening and `km_singer`/`km_extra`
are what you prefer this year; this is where you were *just now*, and it is the shortest of the three.
A break long enough to forget the list forgets the row inside it as well.

**It is the one cookie the server reads and never writes**, because the position is a fact only the
browser has. That is what lets `prefs::set` keep `HttpOnly` unconditional — `km_token` is why that
default is worth keeping absolute — and it is the boundary of the exception rather than a hole in it.

**The list tag is a stamp on the rows, not a second reading of `km_browse`.** A search swaps `#list`
without a navigation, so an anchor can outlive the list it was captured in; and `km_browse` is
rewritten by that same swap, so by the time the stale anchor arrives the cookie beside it already
describes the *new* list and the two would always agree. Eight hex digits of FNV-1a over the browse
state, the same function and the same caveat as `ASSET_VERSION`: it is not defending against anything,
it only has to differ when the list differs.

**A capped prefix, ten pages.** The anchor's index is rounded up to a whole page, so the row is present
with at most a page below it and `Load more` carries on from a boundary. It deliberately does *not*
replay however many times somebody pressed that button: ten pages loaded and then a flick back to row
ten comes back as fifty rows, which is what they could see. Five hundred `<li>`s is about half a
megabyte and a few hundred milliseconds of layout, once, on a navigation that was happening anyway;
past the cap the row is not in the window and you land at the top. **Ten flicks reach that cap**,
because [`The list carries on by being scrolled to`](#the-list-carries-on-by-being-scrolled-to) asks
for a page without being pressed, so the number to raise it to is one somebody has measured an
evening against. **The alternative was a window** — one page starting at the anchor, with a
`Load earlier` button above it — which is O(1) at any depth and needs no cap. Declined because it invents a control that does not exist, makes the count beside
the filters read oddly, turns a flick upwards into a button press, and prepending rows is the clamp
hazard run backwards.

**Five ways it cannot work, and all five land on the top of page one**: no JavaScript, a first visit,
a row that has left the list, a corpus that changed underneath, and a list scrolled past five hundred
rows. None is a failure state. The one honest cost is that a cookie is per-browser rather than
per-tab, so two tabs open on the remote share an anchor — `km_browse` has that property too, and the
consequence is landing on a real row rather than losing anything.

## A queued row comes back

**The badge a row's `+`, `↑` or `▶` leaves behind stands in front of the buttons for three seconds and
is then taken away, rather than replacing them for good.** A badge that holds the slot means a song
just queued cannot be moved up with `↑`, started with `▶`, or taken out of a folder with `✕`, and
nothing brings the buttons back short of running the search again — one press costing a row every
other thing that could be done to it for the rest of the evening.

**What makes three seconds affordable is that the badge is not the only confirmation**: a toast names
the song by title on the same response, so what the badge adds is a second statement of a thing
already said.

The mechanism is spread over four files and each part is inert alone: the press answers with
`afterbegin` rather than `innerHTML` so the buttons are still there underneath, `:has` in the
stylesheet hides them for exactly as long as the badge's animation runs, and `live.js` removes the
span when it ends — an animation used as a clock rather than as decoration, which is why the
`prefers-reduced-motion` branch shortens the toast's and holds this one at its full three seconds.

**A refusal answers with the toast alone.** Re-rendering the buttons is wrong twice: the fragment *is*
the slot, so `innerHTML` nests a second copy inside the first and gives the document two elements
carrying one id, and it passes no folder, so a song refused inside a favorites folder loses its `✕`
until the list is drawn again.

## A row is two lines, and nothing on it is a label

**Title on the left with `#1019` hard right; artist and duration on the left with the buttons hard
right. Two lines, no third, and no media-type pill.** Both remotes.

**The number is on the title's line, not the artist's.** `.song-sub` is a single line with
`white-space: nowrap` and an ellipsis, because a row is two lines and the second is not allowed to
become three — so a number at the end of `artist · duration · #number` is cut off behind any
reasonably long artist name, and the row prints `Dire Straits · 5:4…` while withholding the one thing
on it a person standing at the machine actually needs. **`.song-name` wraps where `.song-sub`
truncates**, so a long title takes a second line and the number stays whole beside the first.

**Above the buttons rather than leading the title.** Leading it is what the printed book does —
`ARTIST | CODE | TITLE` — but a book is read across a page and this is read down a phone: putting the
code first indents every title by five characters of something nobody is scanning for, and the eye
looking for a song is looking at titles.

**The artist is level with the buttons, not with the number.** `align-items: center` puts the artist
where somebody reading a row expects it: beside the controls that act on it.

**No `video` pill.** A singer does not choose a song by its container, the pill costs width on every
row in the list, and the expectation it would buy is already met where it matters: the player card
draws transpose, tempo and the guide melody **grayed out** for a song that refuses them, per
`Two remotes`. A label on the list is the same fact stated early, less usefully, and in the one place
width is scarcest.

The two `.song-actions` groups are wrapped rather than changed: `#song-<number>`, the `afterbegin`
badge swap and the `:has` rule that hides the buttons behind a badge all key off those elements and
their children, so a new parent is invisible to all three. See `A queued row comes back`.

## A song links out to YouTube

**A search URL built from artist and title, never a stored link — and absent rather than broken where
the title is not worth searching for.** Nothing in the workspace holds a URL: no catalog, no package,
no `SongDto`, and adding one would be a field somebody has to fill in for a hundred thousand songs.

The judgment about *which* rows get one is `km-package-builder`'s, ported rather than reinvented, so
the tool that curates a corpus and the remote that browses it do not send somebody to two different
pages for one song: with an artist, always; without one, not if the title has no spaces and no
lower-case letters, which is the shape of a truncated 8.3 filename rather than of a title. That rule
is measured against the corpus rather than guessed — it is full of `CORCOVAD` and `AMD0123`, and a
search for one of those finds nothing at all. Of 39 songs in a test package, 38 got a link and the one
that did not was titled `ABBA` with no artist.

**The link can dead-end and that is accepted**: a machine under a television may sit on a LAN with no
route out. It is a link and not an action, so the phone says what happened.

## Clearing the search box

**A real ✕ beside the field, not the one `type="search"` puts inside it.** Mobile Safari draws no such
button at all, and where one *is* drawn it empties the field and fires no event htmx listens for — so
the rows below go on showing the results of a search whose words are gone, which is worse than having
no button, because the page then looks like it answered. The attribute stays, for `enterkeyhint` and
the keyboard's action key.

The button replaces the whole `#browse` block rather than just the list, because the field's value is
server-rendered and swapping only the rows would empty the results while leaving the words sitting in
the box.

**A form here must not carry an `hx-vals`.** htmx inherits that attribute into every descendant and
appends it to the `hx-get` URL, so a control inside such a form asks for `fragment` twice and is
refused with a 400 before a handler sees it. The fragment a control wants lives in that control's own
URL.

## Nothing detects "worth singing"

**There is no top-songs list, and no automatic one is possible.** The suitability score rates the
*file* — separate channels, lyrics present, lyrics synced, a melody found — and is `None` for every
video song, so a file can score ten and still be nobody's idea of an evening. Because no machine can
produce the list, a person has to, and one authored file shipped for every catalog is either a
stranger's taste imposed on somebody's collection or a file each owner is expected to edit and rebuild
the binary for.

The two things on the offline remote's tab strip that do come from the person holding the phone are
favorites and the A–Z filter — see `Two remotes`.

## Mirroring the catalog

**`GET /api/v1/songs/export`: NDJSON, keyset-paged, with a version that says whether to bother.** The
offline app needs every song, and `GET /songs` cannot give it to them: `MAX_LIMIT` is 500 by
deliberate design for a *search* — a page a person reads — and lifting it there would lift it for
every caller.

So a route of its own, one `SongDto` per line so a client parses a row at a time, paged by
`?after=<the last number seen>` rather than by an offset, because SQLite walks an offset row by row and
a six-figure catalog paged that way costs time proportional to the square of its size.

`catalog_version` is a counter in `library.sqlite`'s `meta` table, bumped **inside the same
transaction** as every install and uninstall, returned as an `ETag` and repeated in the always-public
`/discover` — so "should I re-download a hundred thousand songs?" is answerable in one request that
transfers nothing.

**It is bumped only by an install that actually changed the rows, which is the difference between the
counter working and not.** `install_startup_packages` reinstalls every configured package at every
start of the machine — deliberately, so a package rebuilt with a new column fills it — so an install
that always bumped would move the number on *every start* and the answer to that cheap question would
permanently be "yes, re-download everything". Two installs of two *different* packages both
legitimately move it, so only reinstalling one package can tell the two apart, which is what
`reinstalling_an_unchanged_package_does_not_move_the_version` does. The comparison is a digest of the
package's song rows taken before and after, through `SONG_COLUMNS` — the same list the export sends,
so the two cannot drift about what "changed" means. Not `PRAGMA data_version`, which only moves when
another connection writes and resets on restart.

**It is a public route, permanently**, and the mirror a client keeps depends on that.

## The singer's remote is not gated at all

**Nothing it serves is an admin action, so there is nothing for a guard to refuse.** Every route it
exercises — search, queue, transport, the settings patch that carries key and tempo, the event stream
— sits outside `/api/v1/admin/` and always will. Serving the page at `/` adds a page, not a
permission, and that is true by construction rather than by a check.

**No guard, no `/login` page and no token cookie.** A login page that could never be needed is worse
than none — it implies the person has forgotten something.

**The property a guard would guarantee is a property of the route table instead**: nothing the
remote can do is anything the API would not already have allowed the same caller to do. The
alternative is a route that reaches the machine without going through `km_api::ops`, and that is
what [`One implementation of each operation`](#one-implementation-of-each-operation) forbids.

**The cookie's reasoning survives one crate over.** A browser cannot be told to put an
`Authorization` header on a link, so the owner's page at `/admin/` keeps the token in an `HttpOnly`
cookie and rebuilds the header its guard expects. That is where a login form belongs: on the page
where every control is an admin action, rather than on the page where none is.

## One implementation of each operation

**`km_api::ops` holds what the machine can be asked to do, together with the events each entails, and
both callers go through it.** The half that would diverge is not the operation — it is the
*publishing*. Queueing is one call to the controller; queueing **and telling every open page about
it** is two, and the second is invisible when forgotten: the queue changes, nothing on any phone
moves, and it reads as a stale browser rather than a missing line. It is also what stops the two
answering differently about what a wrong song number means, or what a full queue is called.

## The dev remote stays

**Unchanged, at `/dev/`, and not deleted when the singer's remote arrived.** It is the API's test
harness — the only way to drive `debug/play-file` by hand — and it is desktop-only on
purpose. The singer-facing remote is what `/` serves.

**It is not served by default**; see
[`Dev remote in release builds`](#dev-remote-in-release-builds).

## Where the banks nobody is offered live

**On the owner's page, and on `/dev/` — not on the singer's remote.** The machine knows about
sixty-three SoundFont banks (`Which banks the machine offers` in [`repository.md`](repository.md)),
and without this they are reachable only from a shell, which excludes everybody who has not got one on
a product whose whole design is that it is operated from a phone. They are listed with their terms and
their sizes behind one opt-in parameter, `GET /audio/soundfonts?all=true`, and no new route.

**`/admin/sound` is where it belongs.** Managing what is installed on a machine is the owner's
page's whole subject, and it asks for every bank rather than the offered ones for exactly that
reason. `/dev/` has the picker too, as it has every other control — so this feature does not depend
on a console, which is what makes turning that console off by default cost nothing here. See
[`Dev remote in release builds`](#dev-remote-in-release-builds).

**Not the singer's remote, and the difference is who is holding the device.** A list that long is a
scroll rather than a choice, and every row carries license prose somebody has to read before acting.
Both pages that carry it are laid out for a desktop or a tablet. What that costs is real and is
accepted: choosing a bank is not a thing to do from a phone in a room full of people. It is the price
of the singer's remote being only the singer's — see [`Choosing a bank`](audio.md#choosing-a-bank).

**It removes a bank as well as fetching one.** On a television the folder a fetched bank lands in is
app-private, so no shell and no file manager can reach it, and a bank fetched by mistake — these reach
a gigabyte — would be permanent until the application is uninstalled. The control is on the
installed-banks table, on every row but the bundled one, which is what `SoundFontBankDto.bundled` is
for: that file is unpacked from the build and would come back at the next start, so a control there
could only return a refusal.

## The remote's window, and its portable core

**The offline remote gets a webview window on Windows and macOS and the browser on Linux, on exactly
the terms `The package builder's window` sets out** — same feature name, same two platforms, the same
Linux exclusion for the same libwebkit2gtk-at-load-time reason, `--browser` to decline it anywhere,
`--lan` to decline it too, and a window that *satisfies* `--open` rather than competing with it.

**One difference, and it is in the page rather than in the window: there is no Quit button, and no
*Open in browser* beside it.** `km-remote-pages`'s layout is a singer's phone UI — a body, a banner, a
`<main>`, and a bottom tab bar with a safe-area inset — with no header to put one in. It is
also the one page in this repository that ships on a phone, where a tab reading "Quit" would be
meaningless. Closing the window asks for the identical shutdown Ctrl-C asks for.

**The case that is lost is narrow and written down here so a future report finds the argument rather
than re-deriving it**: a *double-clicked* GUI-subsystem build whose webview cannot be created (Windows
Server, some LTSC images with no WebView2) falls back to a browser tab and then has no window to close
and no console to interrupt, so it must be quit from Task Manager. The escalation, if somebody hits
it, is `km-remote` merging a `/stop` route of its own over `km_remote_pages::router` and naming the
address in the fallback message.

**The window is portrait — 520×900 against the package builder's 1500×900** — because the page is a
fluid single column with no desktop max-width, and at 1500 the tabs stretch across a monitor.

**The remote's core is a crate of its own**, `km-remote-core`, because the owner's original brief has
always said this becomes an Android and iOS app: **a `main` is the one part of a server that does not
travel**, since it reads `argv`, derives paths, installs a log subscriber, prints, and waits for
Ctrl-C, and a phone answers all five differently. Four seams carry the difference, and each is a field
or a phase rather than a `#[cfg(target_os = …)]` — the distinction being that every one of them
differs between *hosts* rather than between platforms, so a `cfg` could not express it even where it
guessed the platform right:

- **The data directory is injected.** `directories` has no Android module and silently takes the Linux
  XDG path, which depends on an unset `$HOME` and falls back to an unwritable `/`.
- **The bound address is readable between binding and serving**, so a port of `0` is answerable and a
  WebView can be pointed at a real URL while a cold import runs.
- **Shutdown is a future the caller supplies.**
- **Discovery is a trait**, because an mDNS browse on Android sees nothing unless Java holds a
  `WifiManager.MulticastLock` for its duration, and on iOS needs a usage description and a prompt only
  the application can raise — the *same* Android build wants a real browse when it holds the lock and
  none when it does not, which is precisely what a `cfg` cannot say.

`km-remote-pages` itself is untouched by any of it, which is the constraint rather than the outcome.

## The offline remote as an Android application

**The offline remote ships as an APK, and it is the same program the desktop runs rather than a second
one.** The owner's original brief said from the start that it "will, in a next step become, a
web-based Android and iOS app"; this is a *requirements* row rather than a technical one because it
makes a phone a supported place to run this product. `crates/remote/km-remote-android` is a `cdylib`
with six JNI functions over `km-remote-core`; `ports/remote/android/` is the Gradle project.

**A Gradle project of its own rather than a module inside the machine's**, because that project
carries SDL's Java verbatim and a guard that refuses to build without the machine's ~150 MB library
staged — a module inside it would make every build of this small application first build that one.

**Both ABIs, but for a weaker reason than the machine's**: there it is compulsory, since every Google
TV device runs a 32-bit OS and loads `armeabi-v7a` alone, whereas a remote is a phone application and
armv7 is only for old hardware — so `--arm64-only` is an acceptable build here and is not there, and
the staging script deliberately does *not* repeat the machine's warning about televisions.

**`minSdk 26` for consistency and not for necessity**: the machine's floor is `libaaudio.so`, which
cpal links unconditionally, and this library links `liblog`, `libdl`, `libm` and `libc` and nothing
else.

**Zero dependencies in the APK** — an Activity and a WebView are framework classes — which keeps the
build offline and is why the back gesture is wired by hand for both eras.

**The one thing Android genuinely adds is discovery.** Sending multicast is unrestricted, so the
machine advertises happily, but **receiving** it needs `CHANGE_WIFI_MULTICAST_STATE` *and* a
`WifiManager.MulticastLock` held for the duration of a browse, or the browse finds nothing and reports
success. Java holds it across `onStart`/`onStop` and the Rust side is untouched; not holding it while
backgrounded is correct rather than a compromise, since a browse then finds nothing and the recovery
logic answers "stay where you are". That permission also makes Android *imply* `android.hardware.wifi`
as **required**, which would filter the app off any device without Wi-Fi, so the feature is declared
solely to un-require it — the same trap the machine's manifest records, found by reading
`aapt2 dump badging`.

## How the Android applications are signed

**One release key signs both, and `KM_ANDROID_KEYSTORE` is where it is named.** The release page hands
these APKs to people, and the debug key is generated per machine by the Android tooling — so an APK
carrying it installs over nothing, and anybody's own debug key replaces it. A key the project holds is
what makes one build the successor of the last.

**One key rather than two**, because the machine and the remote are two application IDs with no
shared-signature permission between them: a second key would be a second thing to hold and back up for
a separation neither product uses.

**Unset means the debug key, and a fresh clone is why.** Another developer's machine and a CI runner
hold no keystore, and a build that required one would make Android the only platform here that cannot
be built without a secret. That is `KM_SIGN_IDENTITY`'s rule in
[`Signing a macOS release`](distribution.md#signing-a-macos-release) read across, including the half
that matters more: every build says which key it just used. **Named but missing is an error**, because
a command that asked for a signed APK and quietly produced a debug-signed one is worse than a build
that stopped.

**What refuses to publish one is `tools/dist/release.sh`**, not the build. A build is watched by
whoever ran it and a report is enough; a release is a page people are sent, so the debug key is
refused there by name. The build's file cannot carry the marker the way a macOS package does —
both keys produce `app-release.apk` and the published name is assigned later — so the gate stands
where the file is renamed.

**Both APKs carry a v3 signature, which is what leaves a way out.** A v2-only APK ties every device to
this one key: replacing it would mean the uninstall below, for ever. With v3 in the APK, Android 9 and
above accept a new key that presents a signing lineage, so a future rotation costs a rotation rather
than everybody's data. v1 is for Android below 7 and `minSdk` is 26.

**An install over an earlier APK is the one thing this costs**, and it is paid once. Android refuses
to install an APK signed with a different key over an existing one, so every device holding a
debug-signed build uninstalls before it takes a signed one.

**What an uninstall takes is where the two differ**, and only one of them is expensive. The remote's
deletes `favorites.sqlite`, the one file in this product nothing can rebuild — a catalog mirror comes
back from the machine in seconds, a collection of favorites built up over a year does not come back at
all. **The mitigation that makes it survivable is the export in
[`The favorites travel as a file as well`](#the-favorites-travel-as-a-file-as-well-and-the-two-are-not-one-feature)**,
which is what makes the change an inconvenience rather than a loss: save a backup, uninstall, install
the signed build, restore. That is a sequence somebody has to remember, so it is written into
[`DEPLOYING.md`](../../DEPLOYING.md) rather than left to be worked out at the moment a device refuses
the install. The machine's uninstall takes its private packages folder and `library.sqlite`, which is a
copy back and a rescan rather than a loss: a package is a file somebody still has.

**Both release build types name a key**, so `assembleRelease` produces an installable APK in either
project and `RELEASE=1` reaches Gradle in both. What a release build buys over the debug one is the
thing that matters for an APK being handed to somebody: `android:debuggable` is off, and anything
holding ADB access can attach to a debuggable application and run code as it. Neither project shrinks
— `minifyEnabled` is false in both, so the ProGuard files they name are inert — and the native library
is optimized under `RELEASE=1` either way.

## The mirror is thrown away when its shape moves; the favorites are refused

**Two databases, two opposite answers to the same event, and the difference is which one can be
fetched again.** A `catalog.sqlite` that is not the current shape is dropped and fetched again — a
mirror is a *copy* of a catalog the machine still has, so the cost is a download. A `favorites.sqlite`
that is not the current shape is **refused and left exactly as it is**, because the row above calls it
the one file in this product nothing can rebuild and that is not a figure of speech. Neither file
carries a version number, so for both the shape is the number: the columns and tables every query
reads. `discard_unless_current` and `check_shape` sit in one crate doing opposite things.

**`CREATE TABLE IF NOT EXISTS` is a no-op against a table that already exists**, so neither check can
be left to `schema.sql`: an older table would stay in place and every later statement fail with
`no such column`. From the outside that is a remote that stopped working and named a column nobody has
heard of. Both run before the schema batch.

**Discarding the mirror takes `meta` with it.** A refresh skips the download when the stored
instance and catalog version match the machine's, so songs dropped with that pair left behind would
answer *already up to date* over an empty table.

This is the rule in
[`A store opens at its current version or is refused`](foundations.md#a-store-opens-at-its-current-version-or-is-refused),
for two stores with no number to compare.

## A database fault is logged, not read out to a singer

**`RemoteError::Failed` is rendered into the failure page verbatim, so whatever goes in it is what a
phone reads out.** A statement and a byte offset are the right thing to *have* and the wrong thing to
*show*: they go to the log, where the person who can act on them will look, and the page gets a
sentence that says where to find them.

**The unique-name refusal is untouched and is why this is not a blanket rule.** That one is not a fault
— it is somebody typing a folder name they have already used — and it is worded for a person already,
with a place under the box to show it. The distinction is the one `views::message_for` draws
everywhere else: a refusal is a sentence, a fault is a log line.

## Where a phone keeps its favorites

**App-private storage, `getFilesDir()/remote`, and nothing sends them anywhere on its own.** The two
databases are the same two the desktop keeps and are kept apart for the same reason:
`catalog.sqlite` is a copy that may be thrown away and rebuilt by a refresh, `favorites.sqlite` is a
collection somebody made. **Two paths take a copy off the device and both are somebody pressing a
button** — a folder as a QR code, and the collection as a file. Neither involves a sync, an account
or a network of ours, and nothing leaves unasked; what the app does on its own is keep the
collection where only it can read it.

The data directory is **passed in from Java**, because the crate that answers this on a desktop ships
modules for Linux, macOS, Windows and the web and nothing else — on Android it silently takes the
Linux XDG path, which depends on a `$HOME` that is normally unset there and falls back to `/`.

**`allowBackup="false"`**, and that is a decision rather than a default: Android's Auto Backup has a
25 MB per-app quota that a real `catalog.sqlite` is well past, so it would need an exclusion rule to
work at all — and it copies files as they lie while both databases are opened WAL, so an
uncheckpointed one restores inconsistently. A backup that silently returns a corrupt favorites
database is worse than no backup. The right protection is an export, not a platform backup — **and
that export exists now**, answering both of those objections rather than working around them: it
reads rows out through SQL on the connection that wrote them, so there is no uncheckpointed WAL to
copy, and it carries the collection alone, so the quota is beside the point. Auto Backup stays off.

## The Android remote holds no foreground service

**None, and the reasons the Go remote needs one do not transfer.** That application keeps a
`connectedDevice` foreground service because a dropped session costs one of its karaoke unit's five
client slots. Nothing here is being held: the machine serves any number of callers, this remote never
names itself to one, and its link is an ordinary HTTP client plus an event stream that reconnects with
backoff. The server is also *in this process* rather than a forked child, so backgrounding cannot move
its port the way a restart moves theirs.

**What that absence buys**, both of them traps that have cost that project real time: the
`connectedDevice` service type requires a second permission from a fixed allow-list or
`startForeground` throws and takes the app down on launch, and `START_STICKY` on such a service
produced an eleven-day crash loop in which the *system* repeatedly resurrected the app into an
immediate crash. Neither is fixed here; both are unreachable.

**What is lost is small and stated**: if Android kills the process while backgrounded, returning is a
cold start of a couple of seconds landing on the first page rather than where you were — and a first
catalog import interrupted that way starts again, because the import replaces the mirror in one
transaction rather than accumulating. Bringing a service back would need this row changed first.

## The offline remote as an iOS application

**The offline remote ships as an iOS app too, and it is the same program again rather than a third
one.** `crates/remote/km-remote-ios` is a `staticlib` with six `extern "C"` functions,
`ports/remote/ios/` is the XcodeGen project, and **`km-remote-pages` is untouched**. A *requirements*
row rather than a technical one, because it makes an iPhone and an iPad supported places to run this
product.

**A `staticlib` where Android has a `cdylib`, and that is the platform rather than a preference**: iOS
will not load an arbitrary dynamic library and forbids `fork` and `exec`, so linking the server into
the app binary is the only shape available — and it is the shape `km-remote-android`'s six JNI
functions were deliberately kept to, so the difference comes out as a calling convention.

**The host state machine is a crate of its own**, `km-remote-host`: it has no platform in it, and a
second copy of the part this repository calls "easy to get wrong" is not a thing to keep two of. The
`Locator` is an argument to `start` rather than `Config::new`'s default, because the two shells cannot
both browse — and a test is a host, so a host chooses its locator rather than a `cfg` substituting
`NoLocator` inside `serve`.

## Finding a machine without multicast

**The iOS remote sweeps the subnet by unicast instead of browsing mDNS, and that is forced rather than
chosen.** iOS has required `com.apple.developer.networking.multicast` for multicast and broadcast
since version 14 and Apple grants it only after a manually reviewed request; without it a browse
**finds nothing and reports success**, forever, which is indistinguishable from a house with the
machine switched off and is the worst shape a fault can have.

`km_remote_core::find::Sweep` asks each address on the local subnet for `GET /api/v1/discover` and
takes the first that answers *naming this application*. It needs only `NSLocalNetworkUsageDescription`,
which is mandatory and whose absence is also silent.

**The camera the share pages use needs no entitlement, and that contrast is the point of saying so.**
`NSCameraUsageDescription` is a usage description granted by the person at run time and reviewed by
nobody; the multicast entitlement above is granted by Apple after a manual request, which is the
whole reason this port sweeps a subnet. Nothing about the scanner reopens that argument, and a reader
arriving at this section is exactly who would wonder.

**Apple's own Bonjour APIs would have worked and were declined**: `NWBrowser` with `NSBonjourServices`
needs no entitlement either, but it puts discovery in Swift — a seventh FFI function to hand the answer
back down, and the one genuinely tricky part of the port becoming the one part `cargo test` cannot
reach.

**What the sweep costs is written on the type**: it assumes the machine is on 8177, where an SRV record
carries a port, so a machine moved off it is invisible and must be typed.

Five rules, four of them from the owner's Go remote, which solved this on the same platform and
measured 21–86 ms on a /22:

- **Walk outward from our own address, alternating up and down** — the machine is usually a few
  addresses away.
- **Stop at the first answer.**
- **Pace the probes**, because a thousand at once looks like a port scan to a consumer access point and
  the one it drops is the machine.
- **Refuse anything wider than a /20** — past that it is an office, and a machine there is named rather
  than hunted for.
- **Bound the concurrency**, which is this port's own rule and is about file descriptors as much as
  politeness: every outstanding TCP connect against a dead address is held for the whole connect
  timeout by the process that is also running the server the WebView is reading.

**Never a guessed /24.** If the interfaces cannot be read the answer is "nothing found", because a
guess covers a quarter of a /22 and the recovery loop would go on being wrong about it every twenty
seconds.

## Where an iPhone keeps its favorites

**`Library/Application Support/km-remote-pages/`, and the two databases are backed up differently.**
Application Support rather than Documents, because the catalog is this device's copy of what the
machine already holds and Documents is backed up to iCloud *and* exposable in the Files app; and
rather than Caches, which the system may empty between launches and which would take the favorites
with it.

The directory is created by Swift and passed down, never derived in Rust — the first of
`km-remote-core`'s four seams, and here the crate a desktop uses for it would not merely guess wrongly
but answer with a path outside the container.

**`catalog.sqlite` is marked excluded from backup and `favorites.sqlite` is not**, which is the one
thing this platform can do that Android cannot: the mirror comes back from the machine in seconds and
the collection does not come back at all.

**A backup saved from the pages goes to `temporaryDirectory` and never to `Documents`**, and that is
what keeps the exclusion above a two-file array. `Documents` is iCloud-backed and exposed in the
Files app, so a file written there would be a third thing the array had to know about — and a
*derived* copy of the collection landing in iCloud is the opposite of what excluding the mirror is
for. A fresh directory per save, too, because WebKit refuses a destination that already exists and
two backups taken on one day carry the same dated name. The file is handed to a share sheet, which
is where a person chooses whether it goes to Files, Dropbox or Drive.

## What a favorite is

**A favorite is a named list a song is in — there is no other kind.** A star that sets a boolean on the
song has no answer to *which favorite?*, and a song can then be a favorite of nothing, which says less
than a set of named lists already says. So there is one mechanism: the lists are the favorites, and a
collection divides by naming more of them rather than by putting one inside another — see
[`A favorite does not nest`](curation.md#a-favorite-does-not-nest). A song can be in as many as you
like, and **the star in the song list asks which one** — one click files it there, the same click
again takes it back out, and a favorite can be created and filled in the same step, because the first
one has to be makeable at the moment somebody wants it.

## A folder travels as a QR code, and the merge only ever adds

**A favorites folder is carried to another phone as a QR code held up between two screens — no
network, no pairing and no account.** Two phones each keep their own collection and, by design,
cannot reach each other: every shell here binds loopback, so nothing else on the WiFi can drive the
karaoke machine, and that stays. A code held between two screens needs none of it, which also suits
the pair of devices this was written for — an Android phone and an iPad, between which neither
AirDrop nor Nearby Share exists.

**The merge is a union, and that is the whole design rather than a simplification.**
`Favorites::add_songs` is the only write either half of this performs and it cannot remove anything,
so merging is commutative and idempotent: there are no tombstones, no clocks and no conflicts to
resolve, and reading the same code twice is a no-op the `(folder_id, song_code)` primary key makes
free. Do not add a delete path to make it "proper" two-way — **the guarantee that nothing is ever
lost is the feature**, and it is what lets the confirm screen have no undo and the done screen no
warning. A merge does go one way, so the done screen offers the return leg rather than leaving
somebody to find out by discovering half their songs missing on the other device.

**Songs this device's catalog cannot show are dropped before the write, not stored and hidden — and
now listed rather than only counted.** A folder's count comes from `favorites.sqlite` while its
listing is filtered through the mirror, so keeping a code nothing can draw would make a folder claim
more songs than it lists. What changed is what a person is told about it: the report names the songs
and groups them by what would fix each, because a song pack that is not installed is something the
owner can go and install and a recording that is in none of the packages they do have is not. A count
alone said only that a number had gone missing. The page names a sample and the log names all of
them, since a phone screen has no room for eleven hundred and a collection somebody is recovering is
exactly the case where the rest matters. The exception is
the case that would otherwise lose somebody everything: a phone that has never reached a machine has
an *empty* mirror, which is this app's normal starting state, and filtering against it would discard
a whole collection at the moment somebody was restoring it. An empty catalog therefore keeps
everything, and the count drift that follows is a state this app already accepts — a favorite is
allowed to outlive the package its song came from — which resolves itself on the first refresh.

**The payload is decimal digits, and the argument that produced that is not the argument that keeps
it.** The reference implementation chose an all-digit string because `rsc.io/qr` selects one encoding
mode for a whole payload, so a single letter anywhere would push hundreds of characters out of QR's
numeric mode — 3⅓ bits a character — into byte mode's 8. That premise is false here: `qrcode`
segments the payload and picks a mode per segment, so a name in byte mode beside its codes in
numeric mode is what it would produce unasked. Three things keep the format anyway. The **validation
is defined over a digit string** — "every character is a digit" is one total, cheap rejection of a
photographed URL or a ticket barcode before any structure is trusted, and the mod-97 check is the
whole payload read as one number. The **saving and the cost are both confined to the name**: three
digits a UTF-8 byte is ten bits where byte mode spends eight, paid on a label of a few characters
against a body of hundreds. And it is a **format shared with a sibling project**, which is the
point: a folder passes between the two programs only if both write the identical string, and
`a_code_this_build_writes_is_the_code_the_sibling_project_writes` pins one against that project's own
encoder rather than against a reading of its documentation.

**Encoding is checked, not trusted.** The header fixes the total length and decoding verifies it, so
a camera that read three quarters of a code fails rather than importing a short folder; a mod-97
digit pair closes the remaining case, a plausible string of the right shape and the wrong contents.
Five refusals rather than one, because the remedy differs for each — a code from the retired first
format needs the *other* device updated, and saying "that did not work" would send somebody to the
wrong phone.

**Past roughly 1,400 songs a folder outgrows any QR**, and the sending page says so and opens its
text box rather than linking an image the handler will refuse — which is a broken icon and no
reason. The typed code is the same fallback a camera that will not start uses, and it is
load-bearing rather than a courtesy: `getUserMedia` needs a potentially-trustworthy origin, so a
phone browsing a `--lan` remote over plain HTTP has no camera available at all and the scanner hides
its own button.

## Sharing is reachable only from inside a folder; a backup only from the Setup tab

**One rule stated twice: a screen never asks a question its own path has already answered.** Sharing
lives at `/favorites/share/{folder}` and is offered by the folder's own header, so there is no folder
to pick when sending and none when receiving — the folder you are in is the answer for both sides. A
backup is the whole collection, so it is offered where nothing has narrowed which part of it you
mean — and every screen under Songs has: inside a folder it would read as backing up that folder, and
on the list of them as backing up the favorites rather than the collection. So it lives on the
[Setup tab](#setup-is-a-fourth-tab-and-it-is-the-narrow-one), which is where the things that are about
this device rather than about a song already are.

**The ✕ leaves to the folder list and the ‹ does not.** Leaving means being finished with the
favorites, which is one place whichever flow you were in and what a restore has just changed; backing
out means the screen you came from, and the two flows are not entered from the same one.

That constraint is what makes the flow five short screens instead of a form. Putting both halves on
one page describes the scheme instead of doing anything, and splitting Send from Receive while still
asking which folder at both ends asks a question the folder already answered. **Which device you
are** is the one question left, and it is the first thing the flow asks — because it is the question
a page showing both halves at once never puts, and the one somebody standing beside another phone
actually has.

**The scanned code is never on screen in the receiving path.** It rides in a hidden field, because
showing it was most of what made this look like a tool rather than a task. What *is* shown is the
name the code carried — not to choose a destination, which being inside a folder already did, but so
that scanning the wrong folder's code is visible before anything is written. A name that disagrees
with the folder it is going into is said out loud and is not an error: merging one folder into
another is a fair thing to want, and names not matching is also what a mis-scan looks like.

**Neither the merge nor the restore redirects afterwards, and neither confirms first.** The write is
add-only and idempotent, so a reload that repeats it changes nothing, and showing the outcome
directly is worth more than guarding against a resubmission that cannot do harm. Every screen in both
flows carries a way out to the folder list beside the step-back that only some of them have, because
these run several deep and backing out one at a time is not what "done" feels like.

**Neither flow is a tab of its own.** Sharing lives under the Songs tab, because favorites is a browse
*mode* rather than a place of its own; the backup is a screen under Setup. `layout.html` is untouched
by both, and the bar is the four it was before either existed.

## The favorites travel as a file as well, and the two are not one feature

**A phone's favorites must be able to leave it — to another phone, and to a file.** That is the
requirement, and it is the offline remote's alone rather than one from the original brief: this is
the only place in the product where somebody's own data exists nowhere else. A catalog mirror comes
back from a machine in seconds; a collection of folders built up over a year comes back from nowhere
at all. Two mechanisms serve it because neither does the other's job.

**A QR code carries one folder and a file carries the collection, and neither subsumes the other.** A
code cannot hold a database of any size — one folder outgrows it somewhere past a thousand songs —
and a file is no use for handing a folder to the person standing next to you. So the two formats are
separate: every decision in the codec exists to fit QR's numeric mode, and none of it applies to
something with room. **They meet at exactly one write**, `Favorites::add_songs`, which is where the
add-only guarantee is stated once for both.

**This is the export `How the Android applications are signed` and `Where a phone keeps its favorites` both
named as the missing mitigation, and it answers their objections rather than working around them.**
Auto Backup was refused on two grounds: a 25 MB per-app quota a real catalog mirror is well past, and
that it copies files as they lie while both databases are open WAL, so an uncheckpointed one restores
inconsistently. This reads rows out through SQL on the connection that wrote them, so it sees
committed data by construction and leaves no `-wal` sibling; and it carries the collection alone, so
an eleven-hundred-song file is under a hundred kilobytes. Auto Backup stays off, and `allowBackup`
stays `false`.

**JSON, and a `kind` field carrying identity.** The format follows `Backing up what a person typed,
and nothing else` in [`curation.md`](curation.md#backing-up-what-a-person-typed-and-nothing-else),
including the rule that matters most: the `format` number is **compared and reported, never
enforced**, because nobody types into a backup and the expensive failure is a file refused at the
moment somebody is trying to get their collection back. What JSON does not give for free is the
identity half — an XML root element is what turns away a photograph handed to a file picker, and
without a discriminator nothing distinguishes this from any other document carrying a `folders`
array. Hence `kind`, required. **Strict about identity, forgiving about shape**: a blank folder name,
a mistyped code, the same folder written twice are all recovered, because a file somebody edited by
hand is exactly the case worth recovering; not ours, damaged, or empty refuse, because half a restore
is worse than none.

**Titles and artists ride along and are ignored on the way back; the package and the content hash
ride along and are read.** A backup that says nothing about what is in it cannot be checked, cannot
be diffed, and cannot be salvaged by hand when something else has already gone wrong — that is what a
title is for, and it is why a title is never trusted to identify anything. The other two are there to
be trusted: they are what lets a collection restore onto a machine that banks the same package
differently and still land on the right songs, which a code alone cannot do. **The identifying pair is
written even for a song this catalog cannot name**, from what the favorite itself holds rather than
from a lookup that failed — that is the case a file most needs to carry, because a favorite of a
package that is not installed today has no title to write and its hash is the only thing that will
ever find it again. All of which is why **export preserves where import filters**: a song the catalog
cannot name is still written, because dropping it here would lose it for good, whereas keeping one at
restore is the count drift above. An empty folder is left out of the file *and* out of the count shown
before taking one — a page promising four folders that writes three is worse than one that says three.

**A song code is the rejoin key, held as text.** It is what the machine and the person both call the
song and it survives a rebuilt mirror, which is the same reason `favorite.song_code` is text rather
than the mirror's surrogate id. Held wide on purpose: `SongCode`'s own parser refuses a mistyped
value, and serde cannot refuse one row — so a single fumbled digit would fail the whole document and
take every other song with it. As text, one impossible value is one line in a report. **It is the
rejoin key of last resort rather than the only one** — see the entry below — but it is still what the
document is keyed and de-duplicated by, because two rows naming one code are one membership however
they are labelled.

**The file arrives and leaves as an attachment rather than a multipart upload.** `Content-Disposition`
is the one mechanism all three shells key off to turn a response into a file a device can keep, and
in an ordinary browser it is what makes the page save rather than render. Coming back, the picked
file is read in the page and posted as an ordinary field, which collapses the file input and the
paste box into one handler and one code path — and the paste box is the only path that works with no
JavaScript at all.

## A favorite rejoins its song by content

**A favorite must survive its package being re-banked, re-slotted, or installed on another machine —
and where it cannot, it must be said rather than silently lost.** For as long as a favorite was a song
code and nothing else, none of that held. A code is `bank * 1000 + slot`, and **the bank belongs to
the machine rather than to the package**: `Library::set_package_bank` renumbers every song in a
package because *the number is the identity*, `choose_bank` takes the next free bank when two packages
collide, and a rebuild re-flows the slots. Any of those leaves a collection naming numbers that have
moved.

**The bad half is not the miss.** A stale code that matches nothing lists as nothing, which the
product already accepts. A stale code that matches *another package's* song lists the wrong song, in
the right folder, with no sign that anything happened — and after a re-bank that is the ordinary case
rather than the unlucky one, because the bank the package vacated is exactly the bank another package
is likely to have taken.

**The key is `(package_id, content_hash)`, and it is a candidate key rather than a hopeful pair.**
Two songs in one package with the same hash are a `ManifestProblem::DuplicateContent`, refused both
when a package is opened and when one is written, so the pair names at most one song. Neither half
is assigned by a machine. The hash was already in every manifest and every catalog row, used only
for spotting duplicates; the package id was already on `SongDto` and already mirrored. **No package
format changes and no `.kmpkg` is rebuilt** — this is a use for what is already carried.

**Three rungs, each looser than the one above, and the order is the design.** The pair; then the
hash alone, which finds the same *recording* re-packaged under another id; then the number, which is
what a song whose package recorded no hash has. The second rung legitimately finds several rows —
two packages holding one recording is reported at install rather than refused — so it **picks**
rather than assumes: the lowest number, so that one catalog always answers the same way.

**A missing hash means unknown, never different.** `Manifest::problems` skips a song with no hash in
its own duplicate check, so treating a null as a mismatch would strand every favorite of a package
that carries none. It falls through to the number instead.

**The number rung answers only where a hash cannot contradict it.** A number names one song on the
machine that assigned it and a different one wherever the package landed in another bank, so a rung
that trusts a number alone is the *wrong song, right folder* failure this entry opens with — reached
by the very favorites the two rungs above were meant to protect, and reached routinely, because a
phone carried to a second machine meets it on every song that machine does not have. Where both the
favorite and the song found under its number carry a hash, the catalog can settle it: two hashes that
differ are two recordings, whatever number they share, and the favorite lists as nothing rather than
as somebody else's song.

**The test is the candidate's null, not whether the favorite is identified at all**, and that is the
rule above read the second way. Refusing the number to every favorite carrying a hash would strand a
real case rather than a hypothetical one: a package built before the manifest carried the field
mirrors a null, so a favorite filed against a machine holding a later build of that same package
misses both hash rungs and has nothing left to resolve on. Unknown on either side passes;
only two known and different values refuse. `SongRef::contradicted_by` is the one statement of it,
and the mirror, the machine's own catalog and the page tests' stub all ask it rather than each
spelling it out.

**A folder says what it could not place, where before only a restore did.** A folder's count comes
from `favorites.sqlite` and its rows come from the catalog, so a phone pointed at a machine without
those packages draws a folder claiming more songs than it lists. The count is the honest number — a
favorite outliving the package its song came from is a state this app keeps on purpose — and the
sentence under the rows is what tells the two apart from songs that have gone. It reuses the restore
report's own remedy lines, since a pack that is not installed is something the owner can go and
install either way, and it has a count line of its own because the restore's ends *"so they were
left out"*: nothing is left out of anything here, and the songs come back with the machine that has
them.

**Only while the whole folder is on screen.** The misses are a property of the folder rather than of
a search — an unresolved favorite was never a row a filter could narrow — so a sentence printed
under a search that cut the list to three would be describing something the reader cannot see. It
renders in `#list`, which every search replaces whole, rather than in the rows, which *Load more*
appends to; a note among the rows would be drawn again under every page somebody loaded.

**And it renders above the empty state as well as above a list.** A folder whose songs are all on
another machine resolves to nothing at all, which is the case the sentence is most needed in and the
one a note drawn only beside rows would miss.

**`(package_id, slot)` was considered and is strictly weaker**: it survives a re-banking and not a
rebuild that re-flows the numbers, and the hash is documented as the thing that is *"stable across
rebuilds in a way file paths and numbers are not"*. **Hashing the package id and the content hash
together into one token was considered and is weaker still.** It costs the same on the wire, since
the package id is already there — but it collapses the two rungs into one, so a re-packaged recording
becomes a miss; and it destroys the diagnosis, because `(package "a1b2…", hash "9f3c…")` can say *that
pack is not installed* where an opaque token can only say *no*.

**The song code stays the primary key of `favorite`, and the two columns are nullable beside it.**
Nothing about the add-only merge changes: `add_songs` is still the only write either transfer performs
and still cannot remove anything. The columns are filled by `reconcile` the first time a folder is
drawn against a mirror that knows — **a collection repairs itself by being looked at** — and when a
resolve lands on a different number than the row was filed under, the row is moved, in every folder
holding it, because a song is commonly in several and repairing one at a time leaves the others to be
found broken later.

**So the number is the machine last looked at, and that is the one thing about a collection that is
not the person's alone.** A remote alternated between two machines that bank a package differently
refiles those rows on each visit. It costs writes and not correctness — the pair and the hash find
the song from either side, and the number they repair to is right for the machine in hand, which is
the machine about to be asked to play it.

**The favorites database is added to, never discarded**, where the mirror beside it answers the same
question by throwing itself away. That asymmetry is the one from
`The mirror is thrown away when its shape moves`, and it is not a judgment call: a machine still holds
what the mirror is a copy of, and nothing holds a copy of this. An empty column is the right resting
state rather than a gap to fill before use, because a null resolves exactly as every favorite did
before the column existed.

**The mirror gains the hash and is discarded to get it**, following `tags` for the reason `tags` gave:
it comes off the wire and nothing local can derive it, so adding the column empty would leave every
song reading as *"this package never recorded a hash"* — a state the resolver cannot tell from the
truth, which would silently disable the repair. One re-download of something the machine still has.

**The QR code is not touched.** It is all-decimal to fit QR's numeric mode and pinned byte-for-byte to
the sibling project by a test, and carrying package ids through it would need a new format digit that
sibling phones would refuse. It serves two phones in one room pointed at one machine, where the codes
were never wrong. A format with a package dictionary remains possible and is not owed.

## A remote looks again when its machine goes quiet

**The offline remote keeps trying the address it has, and while that address is not answering it also
browses the network and moves to its own machine wherever that has got to.** Finding a machine once per
start, preferring a remembered address over an mDNS browse, is right for the common case — opening
instantly beats opening in a second and a half — and has no way back: the event stream retries whatever
it was handed for the life of the process, so a remembered address that has gone stale can never be
replaced by the machine actually advertising itself. Reported from a real house: the machine was on
one address and announcing itself, the remote spent the evening on another belonging to a box switched
off in the next room, and nothing on screen suggested where to look.

**Two rules make it recover rather than merely retry.** Looking again happens *in the background*
rather than at startup, so the instant open is kept and the cost is paid only by a remote that is
already not working. And **nothing is remembered until it answers** — the address is written down when
the connection comes up, never when it is picked — which is the half that stops the problem returning,
because an address recorded without ever having been reached is one a browse can never get past.

**Recovering is finding its own machine, never taking another one.** A remote that has met a machine
follows that machine's id wherever it has got to and moves to nothing else on its own — see
[`A machine is known by its id, and its address is a cache`](api-and-network.md#a-machine-is-known-by-its-id-and-its-address-is-a-cache).
The evening the machine is switched off, the remote sits on an address nothing answers, which is
right: the alternative is a remote carried to another house mirroring a stranger's catalog and saying
nothing about it. **A remote that knows of no machine takes what it finds**, and that is how a first
run reaches the machine in the room.

**A machine named on the command line is never wandered away from**, however long it stays silent:
that is somebody saying *that one*, and quietly moving to another would be disobeying an instruction
rather than recovering from a fault. `Naming a machine from the remote` says the same sentence about
somebody typing an address into a page, and adds the half a command line never needed: a way to take
the pin off again.

## A look on the network yields an identity, not an address

**`Locator` answers a list of `Sighting`s rather than the first URL, so the remote learns *which*
machine it found and not merely where.** That is what
[`A machine is known by its id, and its address is a cache`](api-and-network.md#a-machine-is-known-by-its-id-and-its-address-is-a-cache)
needs, and a bare string could never carry it.

**An id is not a name, and only the id crosses this seam.** `The card says which machine, in the
owner's words` refuses to widen a browse to carry a *name*, for two reasons that are both about names:
it would cost a seventh C function, a header, a parity test, both native shells and every paragraph
that says the surface never grows; and a browse-time name is a snapshot of a thing that can be renamed.
Neither reason reaches an id. **An id cannot go stale** — it is minted once in a machine's life — and
**it stops inside `km-remote-core`**: no shell reads one, nothing new crosses the FFI, and the surface
is still six functions.

**And the identity costs no request.** `sync::refresh` opens with `GET /discover` on every connect,
every rescan and every periodic refresh, because that is how it decides whether to download anything,
and the id is in that response.

**The sweep gains more from this than mDNS does.** Its probe already fetched and parsed the whole
discovery document to check that what answered was a karaoke machine and not a printer. Returning a
`Sighting` lets iOS — the one platform that may not multicast at all — anchor on identity like
everything else, and lets a sweep *hunt* one id rather than adopting whichever machine answers first,
which in a house with two is a coin toss dressed as a decision. The cost is that hunting a machine
that is switched off spends the whole six-second budget instead of stopping early; that is bounded and
happens only while the remote is already offline.

## The remote follows its machine, and looks without being asked

**The offline remote listens continuously instead of browsing when it wants an answer, and it moves to
the machine it already knows when that machine turns up somewhere else.** One cause: a look that starts
when somebody wants an answer produces an answer as old as the question.

**`machine_watch` selects rather than ticks.** It waits on the registry's change signal, on somebody
coming back to a page, and on a twenty-second interval. **The interval is still needed**: `Sweep` has
nothing to push and must be polled, a watcher whose daemon would not open needs something to poke it,
and writing the record down is a timer's job. What the push buys is that the case this exists for — the
machine reappearing at a new address — moves the connection in under a second rather than up to twenty.

**A move of the *same* machine is followed whether or not the current address answers.** Watching only
for failure cannot reach the case a home network produces most often: the lease moves overnight,
something *else* answers on the old address, and nothing ever fails. An id makes that case visible, and
`choose` takes it. Where nothing has said what is at the old address, a fresh record stays put and a
stale one moves.

**A pin still beats everything**, including a machine that has demonstrably moved. Somebody typing an
address or passing `--machine` is giving an instruction about an *address*, and following an identity
away from it would be disobeying rather than recovering.

**Android needs no upcall for it.** `Coming back to a page is a reason to try the machine now` has a
resumed page reopening its event stream, and the handler for that calls `wake`; the Activity retakes
the multicast lock in `onStart` *before* the WebView resumes. So `wake` also pokes the discovery watch
and the lock is guaranteed held by the time a query goes out. On iOS the sweep is polled and **only
while the remote is offline**: a thousand connects every twenty seconds from a phone in a pocket that
is happily connected would be a battery bug rather than a feature.

**The card says why the address changed rather than changing silently.** `how` carries the wording
*same machine, new address*, which the card prints and the page pump republishes. No toast: the banner
narrates reachability and the card narrates which machine, and a third element saying the same thing
for the one second they disagree is the churn `The offline banner waits before it says anything` was
written against.

**`choose` is the only policy function.** Two of them, free to disagree about which machine wins, is
precisely the drift this arrangement exists to remove — so a record with no id must behave exactly as a
bare address does, inside the same function.

## Naming a machine from the remote

**The [Setup tab](#setup-is-a-fourth-tab-and-it-is-the-narrow-one) carries a card saying which machine
this device is talking to, how it was arrived at and how many songs the local copy holds — and three
actions: type an address, rescan the network, refresh the song list. Typing an address pins that
machine; Rescan is how the pin comes off.**

**A typed address means the same thing `--machine` means**, so it pins: that is somebody saying *that
one*. **Rescan clearing the pin is a debt rather than an invention** — `find.rs` and `km-remote-host`
both say a host offering pinning owes a way to clear it, and a command line never needed one because
restarting is the way out of a flag.

Two shapes were decided against:

- **Persisting a typed address immediately.** "Nothing is remembered until it answers" is what stops a
  dead address short-circuiting the browse for ever, so a typo is forgotten by the next start and a
  real machine is written down within twenty seconds by the background watch.
- **An error status for an ordinary refusal.** A browse that found nothing is not answered with a 409,
  because htmx does not swap an error response and the button would go dead with the reason nowhere.

The routes are gated on **nothing**, on the same argument favorites are:
asking the machine you are trying to leave for permission to leave it is not a check. Where the locator
cannot browse at all — `NoLocator`, which is what a host without the multicast permission passes — the
Rescan button is **absent** rather than disabled, per `Two remotes`.

## Setup is a fourth tab, and it is the narrow one

**The bar is Songs, Now, Queue and Setup — a gear with no word under it, at a fixed width while the
other three share what is left.** Both remotes. The rule that the bar holds three items and no
fourth is about what a *destination* is, and it survives here as the reason this tab does not look
like one.

**The Now tab holds two halves that are not about the same thing.** The top is what is playing, who
for, and the four rows that adjust it; the bottom is a card about the appliance — which machine, how
it was found, how many songs the copy holds, rescan, refresh, open in browser, the book, the offers
from a scan, and a disclosure for typing an address. That card was longer than the page it was
appended to, and none of it answers a question somebody watching a song play is asking. Splitting on
*the song* against *the machine* is what the page was already doing badly.

**A gear and no label, because it is a utility rather than a place to be.** Songs, Now and Queue are
somewhere you are while you use this; Setup is somewhere you go to make it work and then leave. The
narrow fixed width says that before the page does, and it is the reason a fourth item costs the other
three almost nothing — the objection to a fourth tab was that it would take a quarter of a phone's
bar for something nobody presses twice an evening, and at a fixed 3.4rem it does not.

**The gear is this tab's, and the Queue tab's transport disclosure never took it** — see
[`The transport is folded away, and only on the Queue tab`](#the-transport-is-folded-away-and-only-on-the-queue-tab).
One mark meaning two things in one application is worth less than either meaning.

**What is on it is the appliance and this device.** The machine card; the singer's name; the language;
and the favorites backup. The packages this phone searches are on a Packages tab beside it, see
[`A person hides a package from their own song list`](#a-person-hides-a-package-from-their-own-song-list). What stayed where it was: the player card and its four control rows, which
are the Now tab and the reason this split was worth making; the ⋯ extra-row-actions toggle, which is
an inline switch on the list it switches and would be a worse control anywhere else; and folder
sharing, which is offered inside a folder because being in one is what answers *which folder?*.

**Groups in that order, with a rule between them.** The machine leads because a remote talking to the
wrong one is what brings anybody to this tab; the two preferences follow; the backup is last because
it is the rarest. **The rule is drawn by the group that follows another rather
than by every group**, which is the only shape that survives the machine card being absent — a
separator under the heading with nothing above it is a rule about nothing, and which group is first
changes with the mode. The space is on both sides of the line, so it stands off the row above it as
far as the row below.

**It is on both remotes, and the tab is never gated.** `Capabilities::connection` gates the machine
card *inside* the page, not the page — the online remote is the machine, so it has no card and no
favorites to back up, and what it has is the name and the language. Those two are about whoever is
holding the phone rather than about a machine, so they exist in both modes and a tab that vanished
with the card would leave the online remote no way to reach them. This is not the `Two remotes` rule
being bent: absent-and-not-disabled applies to a control a mode does not have, and the tab is not one.

**The connection dot moved into the gear's corner and stopped being tappable.** It was already
absolutely positioned in the bar's top-right so it would not take width from the tabs; that corner is
now inside the Setup tab, which is the right place for it — the card that explains the dot is one
press away — and `pointer-events: none` is what makes it a mark on the tab rather than a hole in it.

## Rescan offers a machine rather than taking one

**A remote that is already talking to a machine keeps it. Rescan looks, says what it found, and puts a
button beside it — and a remote with nothing answering moves to its own machine and to no other.**

**A press is not a choice of machine.** It means *what else is out there?*, so a remote that knows
which machine is its own moves only to that one, at whatever address it has turned up on; strangers
are listed and left. Pressing one of them is the choice, and that is the only thing on this page
besides the address box that changes which machine this is. **A remote that has met no machine still
takes the first**, having nothing it could be walked away from.

**Looking and switching are two acts.** `Mdns::browse` takes the first service to resolve, which in a
house with two machines has nothing to do with which one the room is using. So a press meant as *what
else is out there?* could take a working evening off the machine it was on, mid-song, with the only
sign a quietly changed address on a card nobody was looking at.

**The pin comes off in every branch, including the one that finds nothing.** That is the promise
`Naming a machine from the remote` makes about this button — it is the *only* way back from "that one,
whatever happens" — and it is independent of whether the connection moves: un-pinned means the
background watch may recover to something better the next time this machine goes quiet.

**Accepting the offer is a fourth action and it deliberately does not pin.** `POST /machine/use` points
at the address and leaves the pin off, where a typed address sets it: typing means somebody saying
*that one*, and accepting what the network offered is not somebody saying it. A remote pinned to a
machine it merely agreed to could never be recovered from by the background watch.

**It is recorded as *chosen*, which is what the card says.** Nothing else points this remote at a
machine it does not already know, so a press on a row is the act that changes which machine this is —
and `chosen` is not `remembered`, so the identity of what answers there is learned rather than
refused.

**An answer about the machine in hand, plus a list of everything else.** `Scan { outcome, others }`,
where the outcome is `Nothing`, `Using`, `Already` or `Kept`. It was four answers each carrying one
address, and that shape produced the fault this entry now exists as much to record as to prevent: two
machines on a LAN, a press of the button, and *"the machine you are already using is the one on the
network"* — with the second machine discarded because there was nowhere to put it.

`Already` is the one that looks like an over-refinement and is not: a browse routinely turns up the
machine you are on, and offering *use this one* for the address printed on the card directly above
it reads as a fault in the page rather than as an option. That is not a reason to say nothing about
the machines beside it: the two are not exclusive, and the list is what says so.

**A move carries the list too.** A remote that had nothing answering, took machine A and was never
told about B is the same fault in its second location — so `Using` answers with the rest as well.

**The machine in hand is recognized by its id where both ends have one, and by its address
otherwise** — `choose`'s own distinction, applied to a list rather than to a move. So a machine that
has merely changed address is not offered back to itself as though it were a stranger; following that
move stays the background watch's job, which is where
[`The remote follows its machine, and looks without being asked`](#the-remote-follows-its-machine-and-looks-without-being-asked)
put it.

**The offers name the machines.** A name costs nothing here: `km_api::discover::Sighting` already
carries one, the registry fills it, `Scan` never crosses the FFI and no host implements a locator.
The reason against it — a browse-time name is a snapshot of a renameable thing — is thin for a value
that lives for one button press and is replaced by `/discover`'s name the moment the offer is
accepted. With **one** offer an address is a complete answer; with three, three bare
`http://192.168.1.x:8177` strings are exactly the "which machine is this?" failure
[`The card says which machine, in the owner's words`](#the-card-says-which-machine-in-the-owners-words-and-learns-it-from-the-connection)
was written against — and the person is being asked to choose between them. A name that is really
the machine's id is dropped, since that is what the registry falls back to and it would be worse
than none.

**The list is sorted and deduplicated where it is built**, by name then address. `Sweep` answers in
whatever order its probes finished, and mDNS and a sweep can both see one machine, so a list that
reshuffled or repeated between two presses would read as a page that cannot make up its mind.

**And it renders outside `#machine`, swapped out of band.** The pump republishes the card whenever the
connection changes, so an offer inside it would be gone within the second. Every one of the four
actions answers with the offer block, empty unless a rescan filled it, because an offer that outlived
the press that acted on it would sit there recommending a machine this device has since moved to.

**`Mdns::look` is not the narrowing it looks like**: it returns the *whole* registry snapshot as
soon as it is non-empty, not the first sighting. The one narrowing left is a cold registry — the
loop returns at the first non-empty snapshot, so a machine announcing 200 ms later is missed on that
one look — which is essentially unreachable from a button pressed minutes after the watcher opened.

## The machine card links out to the machine's own remote

***Open in browser* on the Setup tab's machine card opens the address printed directly above it — the
machine's, not this remote's.** It is a link between the two remotes rather than a way out of a window.
The address it opens is `status.connection.address` and so is the address the card is already showing;
one field read twice cannot drift, and the pump republishing the card keeps both current.

**Present on every host.** Going to the *other* remote is worth doing from anywhere, so all five hosts
draw it; what gates it is having an address at all. Offering a browser to somebody already in one would
be the grayed-out control `Two remotes` refuses.

**An ordinary anchor, and there is no host seam.** Each of the three webviews already sends a URL that
is not its own loopback out to the real browser — wry's `with_new_window_req_handler`, Android's
`shouldOverrideUrlLoading`, iOS's `decidePolicyFor` — and the YouTube link on a song row proves all
three in production. `target="_blank"` is load-bearing rather than decoration: it is what makes wry see
a *new window* request instead of navigating the application's own window away from the remote.

**Drawn while the machine is not answering**, because the banner prints that same address as *retrying*
and a card that hid the link would be pretending the remote does not know where the machine is.

**The tray's own *Open in browser* opens this remote**, which is what somebody wants when the page
itself will not load.

## The offline banner waits before it says anything, then says its piece and goes

**The strip spends about five seconds saying only that it is trying — a spinner and one word, in the
page's own colors — before it turns red and says why. Then eight seconds, and it collapses.** Two
phases of one element, and the class is the phase: the template renders the first and `live.js`
advances it.

**The machine going quiet for a moment is common and mostly means nothing**: a stalled heartbeat on the
Android machine, a phone whose radio has just woken, a television switched off and on again. The remote
is trying again within about a second, so an alert at the instant the stream drops is a red bar for
something that has usually mended before anybody has read it — reported exactly that way, as a red bar
appearing over and over through an evening.

**Quiet rather than absent**, which was the alternative and is the better argument against itself.
Showing nothing at all would remove the churn completely — the strip is in normal flow above `main`, so
appearing and clearing is two layout shifts, and a gray bar shifts a page exactly as far as a red one.
But the reason somebody is looking at the page may well be that a song has stopped, and a spinner is
the honest answer to *is it doing anything about it?* where an empty page is not.

**A real outage is reported for eight seconds**, measured from when the strip turns red rather than
from when the element arrived. The element's life is thirteen seconds and its red life is eight, and no
third constant is the sum of them.

**Then the tab-bar dot and the Setup tab's machine card report the connection.** A television that is off
is the normal state for most of the day, and a red alert across the top of every page is a nag rather
than news. The strip reappears whenever the reason changes, because a fresh element arrives and the
clock is per element, so nothing that is actually new is swallowed. This is affordable because the card
says all of it, in more detail, with three actions attached.

**Hidden, never removed**, which is not decoration: the element is where the next connection event
lands, and `_banner.html` renders an empty one rather than nothing for that reason. And it goes on a
timer rather than on an animation, because `prefers-reduced-motion` can shorten an animation to nothing
and a backgrounded tab may never fire `animationend` — a bar that needed an animation in order to leave
would be the fault it was meant to fix.

## The card says which machine, in the owner's words, and learns it from the connection

**The machine's name leads the card and the address is the quiet line beneath it — and the name comes
from the last `/discover`, not from the browse that found the machine.** A card saying
`http://192.168.1.9:8177` and nothing else answers "which machine is this?" with the one fact a person
in a house with one television does not need and cannot check.

**Where the name is taken from is the whole decision.** The obvious source is the mDNS announcement,
which carries a `name` TXT record and is where `km-package-builder`'s discovery list reads it. That
would mean widening `Locator::browse` to carry the name through `Found`, the connection, the FFI — a
seventh C function — the header that argues the surface should never grow, its parity test, both native
shells, and nineteen places in prose that say "six functions". Rejected for two reasons, the second
better than the first:

- **`sync::refresh` already asks.** It opens with `GET /discover`, on every connect, every rescan and
  every periodic refresh, and that response carries the name. Nothing new is fetched and no round trip
  is added.
- **A browse-time name is a snapshot, and a machine can be renamed.** A name taken when the machine was
  found would be shown for as long as the process lived; a rename moves `settings.json`, `/discover`
  and the advert and would leave the remote calling it the old thing. The refresh-time name is at most
  one refresh old.

**Nothing reaches the native shells, and that is not a gap.** Android and iOS poll
`km_remote_machine()` as a null check — is there one yet — and once there is, the WebView draws this
card. The name arrives through the page, which is the only chrome either shell has.

**A machine with no name is shown by its address alone**, with no heading element rather than an empty
one.

**And the name survives a reconnect.** The connection is rebuilt on every event-stream transition,
which on a box under a television is most of the day, so recording the name and the reachability
through one setter would show a name for the minutes after a refresh and then lose it silently — a
worse failure than never showing one. They are two setters for that reason. The one call that *does*
drop the name is being pointed at a different machine, which is the only one that can mean a different
machine at all.

## Coming back to a page is a reason to try the machine now

**A page opening an event stream asks the remote to attempt its machine at once, and a stream that had
connected starts its wait over.** Two halves of one sentence — *coming back is cheap* — reported
together from an evening with the Android remote: switching away from the app and back always showed
the red strip, recovery took about ten seconds, and pressing a tab fixed it immediately.

**Nobody was asking.** A phone that goes into a pocket freezes the whole process, server and page
alike, so the backoff a resumed remote sits in was measured before the interruption and has no bearing
on what is true now. The precedent is already in this file: `The dev remote comes back on its own` says
of its Reconnect button that *somebody pressing it is saying they think the machine is back, which is a
better guess than the interval a run of failures had arrived at.* A page coming back on screen is that
same statement without a button.

**Doubling is for a machine that is not there.** Applying it to any dropped stream — a machine that
stalls past the idle deadline, a socket that errors — walks the interval out to its cap for a machine
that blinks repeatedly, and each later blink then costs half a minute of *not reachable* for a box
answering again within one. It is what stops a remote hammering an address nothing is listening at, and
it has no business measuring a connection that came up.

**A stream opening is the whole signal, and there is deliberately no route for it.** A page opens one
when it loads, when a tab press replaces the document, and when it comes back on screen — every one of
those is somebody looking at the remote again. A `POST` of its own would have to be called from all
three, would spend a second request against the six connections a browser allows one host, and could
drift out of step with the reopen; here the reopen *is* the request.

**Nothing reaches the native shells**, on the same argument `The card says which machine` makes: the C
surface stays at six functions, both mobile shells and the desktop are covered by the page they already
host, and `Machine::wake` is a defaulted no-op so the online mode — where the machine is this process
and there is no link to retry — needs no edit at all.

**What was decided against**: taking the wake into the wait for a *first* address, which is the
locator's business on its own recovery interval and where there is nothing yet to retry; and widening
the five-second idle deadline, which is refused in the row below.

## Five seconds of silence is still a machine that has gone

**The idle deadline stays at twenty missed heartbeats, and a flap is answered where it is seen rather
than where it is detected.** The Android machine stalls its own state ticker occasionally, and
`run_state_ticker` uses `MissedTickBehavior::Delay` — so a stall of a little over five seconds emits one
late tick rather than a catch-up burst, and lands squarely on the remote's deadline. Raising the
multiplier to forty would swallow that whole class of blink, and is refused:

- **A remote that waits longer lies for longer about a machine that is genuinely gone.** The polite
  close arm of the reconnection loop exists to stop the dot staying green for a backoff's worth of
  seconds after the television was switched off; ten seconds of green is a regression in the same
  place, and the sofa cannot tell the two apart.
- **The derivation exists to make this visible.** `STREAM_IDLE_TIMEOUT` is written as the machine's own
  `STATE_INTERVAL` times twenty precisely so that a machine which stops ticking is loud here. A 250 ms
  heartbeat that stalls for five seconds while HTTP is still being served is a scheduling fault on the
  *machine*, and widening the tolerance is turning off the smoke alarm to stop the noise.
- **It is already fixed where it was reported.** With the wait no longer growing and a wake on return, a
  blink recovers in about a second of backoff plus a handshake plus at most one of the pump's
  one-second ticks — so the banner's grace covers it and what is left on screen is the dot going red
  for a moment.

**What the grace is coupled to is the retry interval, not this deadline.** `BANNER_GRACE_MS` is measured
from the moment the strip is *published*, which is already after the silence has been noticed — so what
it has to cover is one recovery: a backoff, a handshake, and up to one of the pump's ticks. Making the
remote slower to notice silence would not move it; making it slower to retry would.

**The reason string is the diagnostic** and it costs nothing to read before touching any of them: *The
karaoke machine stopped answering.* is the idle deadline, so a machine that stalled; *The connection
dropped (…)* is a socket error, so a network or a radio.

## A remote never holds a phone awake

**Neither mobile remote takes a wake lock, and the screen sleeps on whatever timeout its owner set.** No
`WAKE_LOCK` permission, no `FLAG_KEEP_SCREEN_ON`, no `setKeepScreenOn`, no `isIdleTimerDisabled` and no
`navigator.wakeLock` in the pages either. Recorded because it is true and would otherwise be unwritten,
which is the state in which somebody adds one: the reports that produced
`Coming back to a page is a reason to try the machine now` all read like arguments for holding the
screen, and the answer to every one of them is on the recovery side instead.

**The machine is the opposite case and the contrast is the argument.** That application does keep its
screen on — from SDL rather than from anything in its manifest — because it is a box under a television
with the words to a song on the screen, and a screensaver over the lyrics is the whole failure. A remote
is a phone somebody put down between songs. Holding it awake would flatten a battery to keep a page warm
that nobody is looking at.

**What it costs is real and is paid elsewhere.** With no wake lock and no foreground service — see
`The Android remote holds no foreground service` — Android is free to freeze the process the moment the
screen goes off. That is exactly why coming back has to be cheap: the page reopens its stream, the
reopen replays every fragment and asks the link to retry, and the banner waits before it says anything.

**A song does not change this.** A remote is never the thing showing the words — that is the
television's job, and the singer's remote deliberately has no lyrics view at all — so there is no moment
when this device is the one being read from across a room.

## A viewer chooses the remote's language, and the machine does not choose it for them

**The cookie if there is one, the browser's `Accept-Language` if not, English if neither names a
language this build has.** A phone belongs to one person and a television belongs to a room, so this
is per viewer where `machine.locale` is per machine — two people at one party read one queue in two
languages, and neither has changed anything for the other.

**Negotiation happens before any cookie exists**, which is the half worth stating. A phone handed
round at a party has no history with this remote, and asking somebody to find a language picker in a
language they cannot read is asking them to give up. The cookie only ever *overrides* what the browser
already said.

`km_locale`, a year, beside the four preferences already in `prefs.rs` — read in `Prefs::read` rather
than in middleware or an extractor, because that function is already the first line of every handler
and middleware would have to be installed by each of the five hosts. The owner's `/admin/` pages read
the same cookie, since both are mounted on one origin. **`km-admin` is the surface that is not**, a
fourth program on a loopback port with no remote beside it, so it writes the cookie itself from its
own front door — see
[`…and the third question it asks is what language it is in`](distribution.md#and-the-third-question-it-asks-is-what-language-it-is-in).
The value is built by `km-locale` for both, name and path and life together.

**The picker is on the [Setup tab](#setup-is-a-fourth-tab-and-it-is-the-narrow-one), beside
`Singing as`.** Those two travel together — they are what this remote knows about *you* rather than
about a song — and they went to Setup together when the tab was made, because a preference is a thing
you set and leave rather than a thing you read the queue past. Choosing answers `HX-Refresh` rather
than swapping a fragment: every word on the document changes, including the tab bar and
`<html lang>`, so no target would be right.

**Each option names its own language.** See `The interface has a locale; a song has a language` in
[`foundations.md`](foundations.md#the-interface-has-a-locale-a-song-has-a-language).

**…and so does every fragment pushed to that page afterwards.** A fan-out with no locale in it makes
this decision true of the document a phone loaded and false one second later: a pump rendering its
fragments with no catalog draws `⟦control-key⟧`, `⟦nobody-singing⟧` and `⟦tab-book⟧` over markup
that was correct. Two phones at one party each get their own copies — the pump renders every
fragment once per language, and a page is handed only the ones in the language it is reading. See
[`architecture/remote.md`](../architecture/remote.md#a-pushed-fragment-carries-the-language-it-is-written-in).

**Nothing another process worded reaches the singer.** A refusal has travelled as a code since
`A refusal travels as a code, and the page writes the sentence`; *how this device came to be talking
to this machine* travels as one too, for the same reason and with the same shape — `km-remote-core`
sends `remembered` or `adopted`, and the card says what that means in the language it is being read
in. A code the build does not know draws nothing rather than the wrong sentence: unlike a refusal
there is no true generic for how an address was arrived at, and the line reads perfectly well
without it.

**And the sentence a page composes for itself is the catalog's too.** A count, a plural, a song
title inside a toast: as a `format!` call each comes out in English on a Portuguese page and looks
deliberate rather than broken, so all three are composed through the catalog with arguments, which
is the rule the markup already follows. The one place that has to change shape for it is the machine
card: `394 songs in this copy` is a plural over a count, so that card is *built* per language rather
than only rendered per language.

**And the crate beside this one does not get to write words either.** `km-remote-core` decides that
a machine has stopped answering, that a folder name is taken, that a rescan copied 1,204 songs —
and answering with the sentence rather than the fact would put its US English verbatim on a page in
any language. Being in the same process is what makes that look harmless. It sends a code or a count
and the pages write the sentence, which is the shape `A refusal travels as a code` sets.

**One thing is still the machine's prose and is deliberately never shown.** A 400 is aimed at
whatever built the request rather than at the person holding the phone, so it goes to the log and
the page says the generic failure — exactly what a refusal code this build does not know already
gets.

## A person hides a package from their own song list

**A package somebody never sings from can be left out of their song list, on their phone only.**
Setup has two tabs at the top, Setup and Packages, and Packages lists every package the catalog
holds, each with a box that is ticked while it is shown. A list of packages is a page of its own
rather than a group among the preferences: it grows with the library, and it would push the backup
and the owner's page below a long scroll. Unticking one keeps its songs out of that phone's search, its artist list, and its language
and tag pickers, counts included. Nothing changes on the machine or on anybody else's phone.

**Both remotes, stored in the `km_hidden` cookie for a year.** A hide list is about the person holding
the phone, as the singer's name and the language are, so it follows their rule rather than the
favorites'. It is not a collection somebody owns, which is the reason
[`Two remotes, and which is the smaller one`](#two-remotes-and-which-is-the-smaller-one) gives for
keeping a feature offline. The cookie holds package ids joined by dots. `Prefs::read` keeps only ids
of the generated shape, each once and at most 128, which stays well inside a cookie's size.

**Two ways to reach a hidden song remain, because both name one song on purpose.** A typed song number
still finds its song: it is often read from a printed book, and a number that answered nothing would
look like a broken remote. A favorites folder still lists its songs: somebody chose each one, and a
hide is about browsing, not about undoing that choice.

**An id the form did not list stays in the cookie.** The form posts every package it drew and every
box that is ticked, and only those packages change. A package that is away from this catalog, or from
the machine this phone is talking to, is still hidden when it comes back.

**An empty search says that hidden packages are not searched.** Somebody who hid a package weeks ago
and searches for one of its songs otherwise sees a remote that seems to have lost it.

**The two tabs are drawn only when the catalog holds two or more packages.** Hiding the only package
leaves nothing to search, so Setup keeps its plain heading. The tabs are links to `/setup` and
`/setup/packages`, drawn as the Songs tab draws its modes, and the bottom bar's gear stays lit on
both.

**A box saves when it changes, and the form swaps nothing.** The answer is only a toast, and htmx's
default swap would put the empty remainder inside the form and take every box with it.

**The offline mirror stores each package's name.** A song row carries only its package id, so
`sync::refresh` also reads the public `GET /api/v1/packages` and `Mirror::replace` writes the names
beside the songs. A package whose name did not arrive is listed under its id.

## A row holding a field survives its longest translation

**A field and the control beside it is one class, `.field-row`, and there are four of them**: the
search box and its ✕, the singer's name and *Save*, the new-folder box and *Add*, the machine's
address and *Use*. Four copies of one shape, with only `.machine-change` carrying the two properties
that make it work, draw three buttons' words outside the button — which Portuguese finds and
English does not.

| Row | The word | pt-BR | Before |
|---|---|---|---|
| the singer's name, Setup tab | Save | **Salvar** | 43px button, 48px word — the report |
| a new folder, in the sheet | Add | **Adicionar** | 69px button, 73px word, at every width to 320 |
| the search box, Songs tab | ✕ | ✕ | a glyph, so latent rather than visible |
| the machine's address, Setup tab | Use this one | **Usar esta** | correct since it was written |

**It takes two properties to produce it, which is why it looked like a translation problem.** An
`<input>` is `width: 100%`, so as a flex item its basis is the whole row and it concedes nothing
without `min-width` — every pixel the row is short comes out of the control beside it. A button
would normally refuse, because `min-width: auto` floors it at its own text; but `.icon-btn` sets
`min-width: 2.1rem` for the glyph buttons on a song row, and an explicit `min-width` *replaces* that
floor, and English sits over the same line without crossing it.

**`flex-wrap` is the floor under the fix rather than a media query**, which is the argument
`The A–Z filter is a combobox` already made for the filter row: the breakpoint at `27rem` exists for
one element, and a rule that depends on guessing a width is a rule that is wrong on the next phone.
`.control-row` on the Now tab takes the same wrap for the same reason — *voltar ao normal* against
`reset`, and *Desligada* beside *não achado nesta música*, is a full row in English and over one at
320px in Portuguese.

**The two preference rows do not borrow `.filters`.** That class is the result-count line inside
`#list` and says so in its own comment; a form wearing it gets a line of text's layout, which is how
a preference row comes to have no way to give. They are `.pref-row` over `.field-row`.

## Active tag filters are a strip under the filter row, with a way out of all of them

**The chips saying which tags are on stay under the pickers, last in the browse bar, and become
bordered and accent-coloured with a *clear all* on the end.**

**A grey pill is the wrong weight.** A grey pill on the raised surface is the same weight as a
caption, and these are not a caption — they are the reason these songs are the ones on the screen.
Bordered and in the accent, they read as state rather than as decoration.

**Not above the search box.** The argument for the top is that what a list has been narrowed to is
the first thing to know about it, and the picker is the *next* thing you might do rather than the
thing that already happened. That reasoning is fine and it loses to a mechanical fact: **the strip's
height varies with how many tags are on.** Above the search box, adding or removing a tag moves the
search box down or up the page — and on a phone that box is the most used control there is and wants
to be in the same place every time a thumb reaches for it. Below the pickers the strip grows into a
list that is already scrolling, and moves nothing.

So it sits against the rows it describes. A chip is not a control you go looking for; it is a
statement about what is underneath.

**There has to be a way out of all of them, and the picker is not it.** Its first option is a
placeholder, and selecting it does nothing: `BrowseParams::tags` merges `add_tag` only when
`Tag::parse` accepts it, so a blank is a blank. The search box's ✕ deliberately *keeps* the tags,
because it clears a search. So four tags on meant four presses, in a strip whose obvious candidate
for undoing them did nothing at all.

**Drawn whenever any tag is on, not only for two or more.** With one tag it duplicates that chip's ✕
— and a control that appears only once somebody is already several tags deep teaches nobody it is
there, because being several tags deep is the state you reach by not having noticed it.

**It clears tags and nothing else**: the words in the search box survive, and so does the artist or
folder you are inside. That is the mirror of the box's own ✕, which clears the words and keeps the
tags, and a test pins both directions.

**Still a swap and not a link**, following the rule the chips already followed: a control that only
refilters the list swaps, and only a change of *which* list is a page load. It asks for `#browse`
rather than `#list`, because it changes the strip as well as the rows.

**And the picker asks for `#browse` too.** Riding the enclosing form targets `#list` — the rows and
nothing else — which is right for the search box and for the two `<select>`s that only narrow, and
wrong for this one. **Three faults, one cause.** No chip is drawn, because the strip is never
re-rendered. The picker goes on offering the tag it has just applied and showing it as chosen,
because the browser holds that value and nothing replaces it. And the hidden field carrying the tags
still holds the previous set — so a second choice sends the first one's absence and **replaces** it
rather than adding: `rock` then `brasil` gives the twenty songs tagged `brasil` instead of the thirty
tagged either. That is what looks, from the outside, like the picker clearing everything.

The tags already held are baked into the picker's own URL rather than riding `hx-include`, for the
same reason the removal chip does it: that hidden field is stale at the moment either fires. The
language and the initial go the other way, being live `<select>` values.

**A test rendering correct markup proves nothing here.** Asserting the page the server returns and
never which element the control asks it to replace leaves a whole class of fault a server-rendered
test cannot see. The tests for this assert the attribute.

## The A–Z picker covers artists, and is still offline only

**The letter narrows the artists list by the artist's own initial.** That list is not short enough
to do without one: twelve thousand songs are several thousand artists, and scrolling to `T` for Tom
Jobim is the exact thing the songs list already has a letter for.

**Offline only, exactly as it already was on the songs list.** `Capabilities::initial_filter` is off
online because the A–Z needs the indexed folded-initial column the mirror carries and
`library.sqlite` does not. Turning it on for artists alone would have been possible — that list is
short enough to narrow in Rust in either mode — and would put a picker on the Artists tab and none on
the Songs tab of the same remote, which reads as a bug on whichever tab you looked at second.
One rule for the whole browse bar.

**An artist files under the letter their name sorts under**, because the filter reads
`km_song::text::initial` — the same function the mirror stores for song titles. `Águas` under `A`, a
numbered name under `#`. One alphabet, which is what
[`One alphabet, everywhere`](songs.md#one-alphabet-everywhere) is about.

**Not drawn inside an artist**, where the mode is still Artists but the rows are that artist's
songs: a short list with a heading, and where the chosen letter travels as a hidden field instead.

**Narrowed in Rust rather than in SQL, and that is a measurement.** Both catalogs already hand back
every artist and the page already sizes itself in Rust, so this costs one `initial` call per artist
over a list that has been built. In SQL it would be a `LIKE` on an unindexed column, or a second
indexed column and a migration for each of the two databases — to answer a question about a list
short enough to have been assumed short.

**A letter with no artists under it names the letter.** The wildcard arm answers *No artists — is a
package installed?*, which is a wrong and alarming thing to tell somebody who pressed `Q` on a
corpus with thousands of them.
