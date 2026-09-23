# The screen

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## `km-display` — SDL3

`sdl3` 0.18 with the `ttf` and `build-from-source-static` features.

**Wallpapers.** Full-screen stills cycle behind everything. A **loader thread** decodes the next
image with the pure-Rust `image` crate. It downscales the image to display resolution before it hands
over an RGBA buffer. A 4K JPEG must never be decoded or uploaded whole on the render thread.
The display thread keeps two textures and crossfades.

A missing or empty folder falls back to a generated gradient. The display rescans the folder each
cycle, so files can be added while it runs.

**The fade's clock starts when the image arrives, not when the interval elapses.** The two are not
the same instant, because the decode is asynchronous. The interval running out is what *asks* the
loader for an image. So `Schedule` reports "time to change" and "a crossfade is running" separately.
Between them it waits in `awaiting` and holds the image already on screen at full opacity. Only
`start_crossfade` on delivery begins the ramp.

A ramp started on the interval instead applied to the **outgoing** image with nothing behind it.
`draw_background` clears to `theme.background`, so the screen went to near-black under the 45% scrim.
It stayed there for as long as a decode took. Every automatic change blinked black. The manual one
never did, because it had always relied on `start_crossfade` alone. Two corollaries worth keeping:

- **A `true` from `tick` means "advance the playlist", so it must arrive once per change.** It must
  not arrive on every frame until the image lands, or a slow decode races through the folder and
  shows none of it. That is what `awaiting` is for. It is also why deleting the early fade on its own
  is not the fix.
- **A load that fails must say so**: `abandon_change`, from both the decode and the upload error.
  Waiting is a state somebody has to leave. Otherwise one bad file would stop the cycle for the rest
  of the evening.
- **So must a change nobody could ask for.** `Walls::request` answers whether an image is on its
  way. An empty folder or a stopped loader answers `false`: nothing will arrive, so nothing will call
  `start_crossfade`. This is the one worth testing by hand. The every-cycle rescan lives *inside* the
  same `if` as the request, so a wait that never ended would stop the rescan with it. Images dropped
  into an empty folder while the machine ran would then go unnoticed until it restarted.

**The shuffle is a field on `Playlist`, and the every-cycle rescan is why.** `refresh` rebuilds the
entries from a scan on every change, and a scan comes back in name order. So the first refresh would
sort away an order applied once at startup. A refresh happens every thirty seconds, even on a folder
nobody is touching.

`Playlist` therefore holds a `Shuffle`. `refresh` keeps what survives in the place it had and drops
what has gone. It inserts newcomers at a random index at or after the picture on screen. Never before
it: an insertion in front moves what is showing, and the pass then reaches it twice.

`Shuffle` is a seed and a SplitMix64 step, so the whole order is a pure function of one `u64` and a
test can pin it. `display.rs`'s `seed()` draws the entropy through `getrandom`. That is the seam
`km-display` has always kept. No test could hold a display type to an order if the type reached for
randomness itself. `advance` reorders where it wraps and moves the outgoing picture off the front.
That is the only adjacency a fresh order can produce.

**A change nothing timed still has to stop the timer.** A request from outside the interval calls
`Schedule::change_now`: a song starting, `POST /wallpapers/next`, an upload, a delete. Only `tick`
sets `awaiting`.

Without `change_now`, the interval goes on accumulating under a decode that is already in flight. A
request made a moment before the interval runs out then fires the timer as well. Two images are asked
for and the playlist advances twice. Neither picture stays on screen long enough to be seen.
`start_crossfade` and `abandon_change` clear it exactly as they do for a timed
change, so it adds a way in rather than a state to unwind.

**Both triggers are evaluated every frame**, which is what `||` would not do. The old form
short-circuited. On any frame where the timer fired, the request flag went undrained and the next
frame took it, so one change became two. It survived while only a person could ask; a song starting
asks often enough to meet a tick.

**The gradient goes underneath the images rather than instead of them.** An image fading in has to
fade in out of *something*, and the clear color is near-black. `needs_gradient` asks whether anything
will be **opaque**. An outgoing image always is, and a `current` at full fade is; one part-way through
a fade-in is not. The question comes before the blits rather than after.

The old form asked "did we draw anything" afterwards, and a texture handed over at alpha zero answered
yes. That is why the first wallpaper of a boot rose out of a black rectangle. It should have risen out
of the gradient that exists to prevent exactly that.

Drawing the fallback *after* the images would not fix it either. The gradient would vanish the moment
the fade passed zero, and the image would still be climbing out of the clear color.

**A playlist entry is a `wallpaper::ImageSource`, not a path**, because a zip in the folder is a folder
of wallpapers. Everything downstream cannot tell the two apart: the cycle, the count, the shuffle and
the crossfade. Three consequences:

- **Scanning reads archive directories only**, so the every-cycle rescan stays a directory read.
  The scan orders entries by *archive path + entry name*, so an archive's images sit where the archive
  sits in name order. The scan drops `__MACOSX`/`._*` entries. They carry image extensions and are not
  images. They would put a failed decode between every real wallpaper in any archive made on a Mac.
- **An unreadable archive costs only itself.** The wallpaper folder is a place people drop things into;
  one bad file must not take the rest of the pictures with it.
- **The loader reopens the archive per image** and decodes from memory *by content rather than by the
  entry's name*. It does not hold a `ZipArchive` open. A held handle would go stale under the rescan
  that may have replaced the file underneath it.
- **A pack is a file from a stranger, so the read and the decode both have ceilings.** The wallpaper
  folder takes an upload, and a pack is a shareable format. So neither the archive's declared entry
  size nor the picture's declared dimensions are this machine's own writing. `MAX_ENTRY_BYTES` counts
  the bytes that *arrive* rather than the size the central directory claims; deflate reaches about a
  thousand to one. `bounded` puts `MAX_PICTURE_PIXELS` and `MAX_PICTURE_ALLOC` on the reader before
  it decodes.
- **`image`'s own defaults are not enough on their own.** They cap one allocation at 512 MiB and bound
  no dimension at all. That refuses the absurd while it admits half a gigabyte on a television with a
  gigabyte in it.
- **The dimension cap is the half that also stops the *work*.** `resize_to_fill` walks every source
  pixel on the way to a screen-sized image.
- **Which folder is being watched is not fixed for the run.** `Paths::wallpaper_dir` chooses between
  the owner's folder, an overlay and the shipped set **by contents**. Resolving that once at startup
  leaves the wrong folder watched on every machine whose `wallpapers/` was empty at boot. That is
  every machine before its first upload: it watches the *shipped* folder. A picture added through
  `/admin/` then lands in a folder nothing looks at, for ever, with nothing in any log to say so.
- **`Machine::take_wallpaper_dir_stale` is a second polled flag beside `take_wallpaper_request`.** The
  loop re-runs the choice before it rescans. Before, not after, or the first refresh after an upload
  reads the old folder and the picture appears a cycle late.

**Screens.** *Idle*, *Playing*, *Queue overlay*, *Number entry*, *Connect overlay*, *Error toast*.
Vsync'd loop reading the engine atomics each frame; no locks. The cursor is hidden only while
fullscreen — a window nobody can point at is worse than a visible pointer.

**Where it starts comes from `display.fullscreen`, which defaults to off.** `Options::fullscreen`
overrides it for one run without writing anything back (`--fullscreen` / `--windowed`; `None` there
means nobody asked). A setup program pre-writes `true` for an installed machine, so an appliance is
unaffected and a checkout build does not take the screen. See
[`Windowed mode`](../decisions/interface.md#windowed-mode).

A window that is about to go fullscreen is
created at the *display's* mode rather than at `width`/`height`. The kmsdrm aspect-ratio bug in
[`appliance.md`](appliance.md) turned on that.

**`display.always_on_top` is applied after the window exists rather than as a creation flag.** So
starting in front and being put there by `T` are one code path. A creation flag would leave the key
with no way to undo itself.

That costs one FFI call. sdl3-rs wraps `SDL_WINDOW_ALWAYS_ON_TOP` only
for `WindowBuilder` and `PopupWindowBuilder`, and neither reaches a live window. So
`apply_always_on_top` is an `#[expect(unsafe_code)]` site with a `SAFETY` note. It has the same shape
as `request_vsync` above it, and they are the only two in this file.

Reading the state back is safe (`Window::window_flags`). The code **asks SDL rather than tracking
it**, exactly as `is_fullscreen` does. A compositor that declined the change must not leave the
record disagreeing with the screen. Wayland is the compositor that declines, because a client there
does not place itself in the stack.

The write-back at close mirrors fullscreen's and drops its flag half. `always_on_top_to_remember`
takes no `remember` argument, because no CLI flag declares a run temporary. What it keeps is
`WINDOW_STACKS`, the fifth member of the `KEY_HINTS`/`DRAG_AND_DROP`/`FILE_MANAGER`/`WEB_BROWSER`
family. It asks whether there are other windows to stand in front of. See
[`The window can be told to stay in front`](../decisions/interface.md#the-window-can-be-told-to-stay-in-front).

**Position is on the strip as well as the keyboard, and the labels are seconds rather than arrows on
purpose.** `>>` next to `NEXT` reads as *next song, faster*. Losing the rest of a song mid-verse is
the one mistake on this strip that pressing the same key again does not undo. `SeekBy` carries a
delta and `km-app` clamps it, because the display knows the step and only the machine knows where the
song ends. The clamp stops a second short of that end. Seeking exactly to it trips the watchdog that
advances the queue, which would make `+10s` behave like `NEXT`.

### The catalog summary

- **`CatalogSummary` is pure formatting**: counts in, sentence out. Unit tests cover the plural, the
  empty case and the thousands separator with no SDL anywhere near them.
- **The vertical positions are constants.** A test computes every glyph box from `Theme::glyph_box`
  and asserts the gaps. So a font-size change fails a test rather than overlapping two lines on a
  screen nobody is looking at.
- **`Frame::catalog` is an `Option`, and `None` is not an empty catalog.** It means *nobody has
  told this frame*, which is what `km-app` passes while the library is busy. Drawing `No songs
  installed` for a second in the middle of installing four thousand of them would be the counter lying
  about itself.

Filling it never blocks, because `Catalog::install` holds the library mutex for a whole transaction.
`CatalogCounts` has three outcomes: `Unchanged` is an answer and `Busy` is not. **The `known` version
goes *in* rather than the caller comparing afterwards**, and that is load-bearing rather than a
convenience. `SELECT COUNT(*)` on a whole catalog is a full index scan, and this runs at up to 125 fps.
So the counting has to be skipped when the version agrees. Comparing outside the lock could pair a
version with counts taken the other side of an install.

### The build number in the corner

`Frame::version` is a caller-resolved string, the seam `soundfont_label` and `demo` use. One version
number covers the whole repository, so `km-display` reading its own `CARGO_PKG_VERSION` would print
the same characters. But what the screen states is a fact about the *program*, for the same reason a
bank name and a demo sentence arrive from outside.

`None` draws nothing, which is what `examples/screenshots.rs` passes. Those pictures render through this same `draw`, and a number baked
into them would change every one at every release. The decision is
[`Every program says which build it is`](../decisions/interface.md#every-program-says-which-build-it-is).

**The bottom-left corner of the safe area, and the number pad gives up a line for it.**
`version_origin` is where the text starts. It takes `keypad::margin_x` and `margin_y`, the safe inset
of each axis, which is the corner a television actually shows. The pad above stands on the same two
numbers, so the two read as one object in one corner. `version_reserve` is what the pad subtracts,
and it derives from that origin rather than being named. Three things read that one expression: the
draw, the pad, and the test that holds them apart at every size.

**Above the pad does not fit.** The prompt row runs to within a few pixels of the grid's top edge.
There is no line of room between them, so the number goes underneath. `VERSION_MAX_WIDTH` caps what
any string can claim. That makes the clearance a proof rather than a property of the six characters
this is handed today. `the_number_pad_leaves_room_for_the_build_number` reads both sides off the crate
rather than restating either.

### Leaving the app

*Windowed mode is desktop-only.* Where `FULLSCREEN_IS_FIXED` is set, the loop refuses both F and Esc's
first step. The theme makes the activity fullscreen, and there is no desktop to return to. A remote that
dropped the television out of fullscreen would leave nobody a way back.

**`DisplayAction::Back` goes up one level**: a number being typed is cleared, a loaded song is stopped,
and from an empty idle screen the loop breaks. No level asks for confirmation, because Android's TV
guidelines forbid gating back behind one. The level structure keeps a stray remote press from ending
the evening without needing a prompt. The overlay closes first, which is a fourth rung and exactly
what "up one level" should mean.

**The television disagreed about which key that is.** SDL maps Android's `AKEYCODE_BACK` to `AC_BACK`,
so `AC_BACK` was the only binding. A real Google TV remote's BACK arrives as **`Escape`**. On an
appliance build the Escape arm had no fullscreen step to take, so it went straight to breaking the
loop. One press, mid-song, took the app away. There was *no* diagnostic anywhere, because that arm
logged nothing.

**Two lessons, both about the shape of the mistake.** SDL's keycode table tells you what SDL does with
an Android keycode, not which keycode the *hardware* sends. The comment asserting BACK "arrives as
`AC_BACK`, not Escape" was correct about SDL and wrong about the remote, and stood for weeks. And **an
exit path that logs nothing is an exit path you cannot diagnose.**

Three facts made this a Rust-only change. That matters because SDL's Java is vendored verbatim and
must not be touched:

- BACK reaches native unconditionally through `onNativeKeyDown`.
- Nothing in SDL's C reads `SDL_ANDROID_TRAP_BACK_BUTTON`, and it is better left at its default. There
  the system performs its own back rather than BACK doing nothing at all.
- A clean exit is just `SDL_main` returning.

The *process* lingers afterwards, which is ordinary Android behavior: `finish()` ends an activity, not
a process.

### Pixel space is the one invariant to respect

The window is created with `high_pixel_density()`, so on a Retina panel the backbuffer is the native
pixel size. That splits window points from backbuffer pixels, and everything downstream is laid out
in **backbuffer pixels**:

| Arriving as | Converted by |
|---|---|
| Layout and hit-testing | already pixels, from `canvas.output_size()` |
| Mouse events — *logical window* coordinates | `window_to_pixels(x, y, density)` |
| Touch events — *normalized* 0..1 | `x * width`, `y * height` |

**Getting this wrong is silent**: a click lands somewhere the user did not press, which no geometry
test can see. A keypad unit test builds the pad for a 2× backbuffer. It asserts every key still claims
a click at its own center, and it fails on the first key if the density scale is dropped. Both the
flag and the scale are no-ops on Android and Windows, whose window coordinates are physical pixels to
begin with.

**Fonts follow the backbuffer.** The display rebuilds them on `PixelSizeChanged`, but only when one
of the three rounded point sizes actually moves. That keeps a drag-resize from reopening font files
every frame. A failed rebuild keeps the old fonts: wrongly sized text beats ending the singing.

### The connect panel

The remote is useless if nobody can find it, so the address is a first-class UI element, not a log
line. **The bind address is not the reachable address.** Bound to `0.0.0.0`, the app must discover its
own LAN IPs. It enumerates interfaces and keeps IPv4 where up, non-loopback and non-link-local, and
it prefers private ranges. It re-resolves them every 30 s, since DHCP and Wi-Fi roaming change them
while the app runs.
`km-api` owns this and `GET /api/v1/discover` returns the same struct, so the two stay consistent.

The idle screen shows the URL in large type with a **QR code** beside it, rebuilt only when the URL
changes. Pressing **I**, or the first few seconds after startup, shows the same panel during playback.

**Three sizes, not two, and the third is a different panel rather than a smaller one.** `PanelSize`
is `Full` (the idle screen), `Overlay` (`I`, and the greeting) and `Standing`. `Standing` stays up for
the length of a demo song and holds a QR code with the address as its caption and nothing else. One
expression anchors all three to the bottom-right corner. The corner is where this screen puts the
address, and a size that moved would be a second rule about where to look.

**No panel's two axes come from the same place, and none of the three is a fraction of the screen on
both.** `Standing` is its code's height by its caption's width. The height is `STANDING_QR_SIDE`,
because a QR is a square and size is what makes one scannable. The width is sized to
`STANDING_URL_CHARS`, which is 28. That is `http://255.255.255.255:65535` and a real ceiling, since
`km_api::connect` produces IPv4 URLs and no hostnames. Sized on both axes from the code, it printed
`http://19…` at 720p.

`Full` and `Overlay` are `Card`. The height is their rows added up: headline, address, three detail
lines and air, all off `Theme` in pixels. Only the width stays a fraction, because `PanelSize::rect`
answers callers holding no panel. What a text panel's width decides is how much of a URL fits on a
line.

Until 2026-09-07 those two were `0.26h` and `0.18h`, with rows at fractions *of the box* over padding
taken from its width. The last detail line was drawn below the panel's own bottom edge.
`every_row_of_the_connect_panel_stays_inside_its_box` makes that a failure now. It covers every
combination of an address and 0..3 lines of detail. The rows are centred in whatever room the box
has, and each of those combinations places them differently.

Dropping the headline and the detail lines buys `Standing` a code **larger** than the overlay's in a
card two thirds the area. `the_standing_panel_holds_a_bigger_qr_than_the_overlay_it_replaces` stops
that quietly ceasing to be true. It reads `Card`'s own QR side rather than re-deriving it.
`OVERLAY_QR_SIDE` is capped under `STANDING_QR_SIDE`, so a taller overlay cannot grow a code out from
under the claim. `QR_MAX_WIDTH` is the other cap: a square sized from the height is 384 pixels in the
594-pixel panel of a 1080×2400 phone.

With no address, `Standing` draws nothing at all. The other two sizes give an honest one-line
explanation, and that is exactly what `Standing` has no room for. Those two give the whole panel's
width to that explanation, since a state with no address has no code to keep clear of.

`ConnectInfo::has_url` exists for this. The playing screen asks once a frame whether the standing
panel will draw anything, and `panel()` allocates four strings to answer. A test pins the two
together over every state there is.

**Honest failure states, because a wrong URL on screen is worse than none**:

- Loopback-only says so, plus the one-line fix, and never shows a LAN URL that will not work.
- No usable interface says so.
- A port already in use shows the actual error rather than silence.

**More than one candidate IP shows the preferred one and says *how many* others there are** rather than
naming them. A list for an operator to pick from would cost three wrapped lines on a Windows
developer's box. Those lines hold WSL, Hyper-V and Docker addresses that nothing off that machine can
reach. They would also truncate the one address the panel came to show. The losers are still in the
startup log and in `/discover`. Nobody standing in front of a television is reading four URLs off it.

**Text is SDL3_ttf without HarfBuzz, which is a limit on *shaping* and not on coverage.** Latin and
CJK both draw; Thai, Arabic and Indic do not, and neither does right-to-left. There is **no
`TextRenderer` trait**, and nothing here is behind a seam. An earlier version of this note claimed
one, and the claim outlived the intention. Adding a shaper means turning `no-sdlttf-harfbuzz` off. It
also means fixing the MSVC debug-CRT link failure that flag exists to avoid, not swapping an
implementation.

**Coverage chooses which faces go behind, not list position.** Each candidate file carries a mask of
the scripts it answers for: Kana, Simplified, Traditional, Hangul. `pick_fallbacks` takes one only
when it answers for something nothing already taken does. Two slots therefore hold two scripts, and a
face that answers all four ends the search alone. Both platform lists were wrong before this rule
existed, in opposite ways, and the decision entry says how.

**CJK arrives through `TTF_AddFallbackFont`, and three details make it work.**

- `Fonts` opens one fallback face *per size* per file, because a `TTF_Font` is bound to its point
  size.
- It holds them in a `fallbacks` field declared **last**. Field order is drop order, and SDL3_ttf
  keeps a borrowed pointer to each.
- The attachment happens at load, before the face has rendered anything, because **SDL3_ttf caches
  rasterized glyphs per face**. Attach to a face that has already drawn `.notdef`, and the call
  returns true having changed nothing.

That last one is why the display *rebuilds* its fonts when it first meets a CJK string. It does not
amend the ones it has, and it reuses the same seam a resize goes through. `TextCache::saw_cjk` is
what notices, being the one place every drawn string passes.

## The window title

`display.rs` creates the SDL window, and no handle to it leaves `run_with`. So the frame loop is the
only place that can push a title. It reads `snapshot.now_playing` at the top of every frame already.
That is why the push sits there rather than on the API's `SongStarted` broadcast. Taking the broadcast
would mean a `try_recv` per frame for a fact the frame has in hand.

**The guard compares the title and artist, not the string they compose.** `SDL_SetWindowTitle` is a
round trip to the window manager wherever one has to agree. At 60 Hz the composed-string form would
allocate and format a title per frame to discover that nothing moved. `title_changed` answers from the
pair `titled` holds. So the steady state does nothing at all, and the work happens on the handful of
frames where a song starts, changes or ends.

`titled` is a local of `run_with` rather than a field, because `run` opens a second window when a
display goes away and comes back. `set_title` fails only on a title carrying an interior NUL. The
loop warns about that and updates the pair anyway. So it attempts a title SDL will not take once,
instead of on every frame for the rest of the evening. Android has no window furniture and the call
is inert there, the same as `set_window_icon`.

## The now bar

- **The four buttons are `_transport.html`**, nested by holding the child struct as a field, which is
  `views.rs`'s own idiom. `TransportBlock` carries `target` and `query`, two halves of one fact: which
  fragment answers a press. Constructors set them together, because setting one and forgetting the
  other swaps a whole player card into the top of the queue.
- **The bar is a second SSE event rather than a second target.** One event *can* reach two elements,
  but it carries one rendered fragment and these two want to look different. The pump's
  markup-comparison gate covers both **because the bar is a strict subset of the card**. A card whose
  markup did not change did not change the bar. That is the property to preserve if anything is ever
  added to the bar that is not also on the card.
- **`?fragment=nowbar` never decided anything, and a test still pins that.** The remote is not gated
  at all now, but a press from the queue answers exactly as the bare path does. A query string able to
  change what a press means would be a way round a rule rather than a spelling.
- **The progress bar is deliberately not repeated.** `_position.html` exists precisely because the
  position is the only thing that moves; a copy in the bar would be a second fragment republished every
  second.

## The browse bar

**The A–Z strip is a `<select>`, and the interesting part is which rule that broke.** The bar stands
on one rule: anything that changes *the bar* is an ordinary link, anything that narrows *the list* is
an htmx swap. The strip was in the first group because 27 links drew "which letter is current" twice:
as the list and as an `active` class. A swap that leaves the bar alone could put the two out of step.
A form field looks like bending that rule and is not. **A `<select>`'s value is the highlight**, the
browser owns it, and there is no second drawing.

Two details are easy to get wrong:

- The digit bucket's option **value** stays `#` (only the label moved), because bookmarks and the
  cookie carry it.
- The picker is not drawn everywhere the initial applies, so views without it keep the hidden input
  the select replaced. Without that, a search from inside an artist would silently drop an initial
  chosen on the songs list.

### A feature whose only trigger is missing looks exactly like a feature that is off

Take a preference read on every request and branched on in a template, with **nothing anywhere
setting it**. Everything behind it is markup that cannot be reached. Meanwhile the template's own
comment goes on describing the control in the present tense.

That is worth a paragraph because of *why* it survives. A capability that is off renders nothing, so
there is no blank space and no dead control to notice. The tests asserted what the rows contain, and
that was correct for `extra == false`. **The thing that would have caught it is a test asserting that
some request, somewhere, turns it on.**

The toggle is a parameter on `GET /` rather than a route of its own. A `POST` would have to re-render
the browse block anyway. It would also be a route on this page that reaches no machine at all, and
every other one does.

### A failed move is not a failure, and an idle machine is where that shows

**`▶ now` is three calls where `↑ next` is two**: queue, move to the front, then end what is playing.
The handler tries `Skip` first with `Play` as the fallback, because the machine answers `Unavailable`
to a skip with an empty deck. Reading the state first would be a fourth round trip. It would *also*
still race the song that ends between the question and the answer.

**The song at the front of the queue is what keeps that refusal coming.** A skip into an empty deck
starts a demo song where demo mode is on. A demo starting here would answer `Ok`, leave the fallback
unrun, and put a `Playing now` badge over a song still waiting. The queue is one of `why_no_demo`'s
conditions, and this handler has just filled it. So the refusal the fallback needs is the one the
machine gives.

`a_skip_with_somebody_waiting_refuses_rather_than_starting_a_demo` asserts it. The order of these
three calls is now load-bearing for a reason a reader of this handler cannot see.

Running it found what neither the plan nor the tests had. **With the machine idle, ▶ answered "Queued
…, but it could not be moved up" for a song that was at that moment playing.** Adding to an empty
queue wakes the machine and takes the song straight to the deck. So the `entry_id` just handed back
names nothing by the time the move runs. It is not an edge case: it happens **every time somebody
uses a machine that is not already playing**. That is the normal state of one when a phone is picked
up.

`is_on_the_deck` asks the one question that separates the two, and three things about it:

- **It is asked after the move, not before.** Reading first would answer about the moment before the
  song was added, and would leave the same race open with a stale answer in hand.
- **It matches on the song number**, so queueing a song that is already playing reads as having reached
  the deck. A real ambiguity, resolved the kind way.
- **The two paths cannot be merged**: if the song is already on the deck then `Skip` would skip the
  very song that was asked for. Only the moved-to-the-front path skips.

**It is the elevated half of what a row offers.** It was once gated as `transport.skip` rather than
as `queue.add`, because the guard answered with one route id per request and this action does both.
Nothing is gated here now; what survives is the judgement that interrupting is the elevated half.

### htmx inherits `hx-vals`, and `hx-disinherit` does not stop it

Four facts, none derivable from this repository's code, all measured against the vendored `htmx.min.js`
2.0.4. Together they are why two buttons could be written, reviewed, rendered correctly and do nothing
at all.

- **A parent walk using a plain `getAttribute` inherits `hx-vals`.** A child's own keys win and an
  ancestor's fill the gaps.
- **`hx-disinherit` does not reach it.** The disinherit-aware lookup is a different pair of functions;
  `hx-vals` never goes through them. So the obvious-looking fix is a no-op that would have shipped the
  bug wearing a comment saying it was fixed. The only in-htmx escape is a literal `hx-vals="unset"` on
  the descendant.
- **For a GET, what was collected is appended to the `hx-get` URL with no de-duplication.**
- **`axum::Query` over a derived struct answers a duplicate key with 400**, and htmx does not swap on a
  non-2xx, so the button is silently inert.

**Keep the 400.** A lenient last-wins extractor would have swapped the whole browse block for a bare
list fragment and taken the bar off screen. That reads like a layout bug rather than a request one. A
dead button is the better of the two failures.

**The rule governs anything added to that form. The form's *fields* are the state its buttons must
inherit; the form's *URL* is the part they must not.** So `fragment=list` lives in the form's own
`hx-get`. A hidden input is the wrong repair for the same reason spelled backwards.
`hx-include="closest form"` would pick it straight back up and rebuild the identical duplicate.

**No test in this crate can see any of this**, and that is the part that matters for the next one.
The toggle's own test types the URL in directly, and that is precisely the URL a browser never sends.
The guard left behind is on the *attribute* instead.

### The neighboring bug, and what the pair share

The clear button also dropped the initial. `clear_href()` leaves the language and initial out and lets
`hx-include` carry the live values. That is sound for a `<select>`, and the initial is only
*sometimes* one. Where the picker is not drawn, the initial travels as a hidden input. That field had
no id, so the selector matched nothing. It was not cosmetic: the initial is a real filter in both
places, so clearing a search there came back with the wrong rows.

**Putting the initial into `clear_href()` instead would have reintroduced the bug above.** Where the
`<select>` *is* drawn, `hx-include` sends it too, so the URL would carry `initial` twice and earn the
same 400. Two adjacent controls have two ways to send a parameter, and the repair for each is the one
the other rules out.

**What they share is the question worth asking of any control in that bar. For each parameter it
sends, is there exactly one source? And is that source present in every view the control is drawn
in?**

**"Verified by running it" has to name which control was pressed.** A control inside a form carrying
an inheritable `hx-vals` has never worked in a browser at all. So a claim that survives is one where
pressing it was not what was checked.

## The on-screen keypad

`km_display::keypad` is pure geometry: a layout for a screen size and a hit test against it, no SDL
and no state. It resolves to the same `DisplayAction` a key press does. So touch and a keyboard are
the same thing by the time anything acts on them.

- **Telephone order**, not calculator order, because a song number is read aloud and typed the way a
  phone number is. **Android-only by default**, under a setting a `settings.json` can override either
  way. The whole mechanism is one line in the frame loop, and `km-display` knows nothing about it.
  That is the property to preserve.
- **The transport strip appears for six seconds after a press and then gets out of the way**, because
  it sits over the lyrics. While hidden, a press reveals it and does nothing else. Acting on that
  press would mean skipping a song because somebody brushed the screen.
- **`Ctrl+F12` suspends that timer for whoever is working on the strip itself.** `strip_visible` is
  a named function beside `keypad_visible`, and for its reason: a test asserts the real composition
  rather than a copy. The pin *wins over* the deadline without clearing it, so unpinning hands back
  whatever was left, and a press while pinned still pushes it out. The flag is a loop local, unlike
  the frame overlay's, which lives on the machine so three pages can move it. Only the keyboard in
  front of the strip reaches the pin, and it deliberately does not outlive the process. See
  [`Ctrl+F12 holds the transport strip still`](../decisions/interface.md#ctrlf12-holds-the-transport-strip-still).
- **The melody key is absent when detection abstained**: a control that would mute an arbitrary
  instrument is worse than no control.
- **Labels are words the bundled font can draw**: Latin-1 plus a named handful, `words::is_drawable`.
  The fonts are Latin-coverage only, so `⏎` or `⏸` would render as a blank box. The rule was
  `is_ascii()`, and that was never what it meant: this screen already drew `…`, which is not Latin-1.
  The test now reads every message in every catalog rather than the label literals.
- **A label is a message id and a digit is not.** `PAUSA` and `TOM +` are what the strip says on a
  machine set to `pt-BR`. `7` is `7` everywhere, so the number pad draws its digits directly rather
  than through a catalog somebody could get wrong. **The action is what the table names**, and the
  label follows from it. Deriving the action by matching the printed word made the word a control
  value. Then `CLR` could not be translated without changing what the key did.

The rendered frames caught three things that the geometry tests could not. That is the argument for
having both:

- The pad was first anchored bottom-**right**, straight over the connect panel's QR code. A test now
  names the panel's right-anchored 55%, so the coupling is not silently broken.
- Keys were sized from the screen *height*, giving a portrait phone 264 px keys on a 1080 px wide
  screen.
- Every label rode low in its key, because `draw_text` takes the top of the text rather than its
  middle.

**`FingerDown` and `MouseButtonDown` are separate arms on purpose**: touch coordinates are normalized
and mouse coordinates are pixels. Treating a finger's `x` as a pixel would put every touch in the
top-left corner. And on Android touch is the only input there is.

## The lyric timing offset

**The problem.** The highlight is exact against the *audio* and not against the *screen*. A
television adds tens of milliseconds of panel processing. A rig that takes audio out of the HDMI chain
early shortens the audio path and leaves the video path alone. Mixing microphones in hardware requires
exactly that.

**The rule that shapes the design: this may never delay audio.** An AV receiver's lip-sync control
fixes late video by delaying sound, and on a karaoke rig the sound contains the singers. So the offset
moves *when the highlight is drawn* and nothing else. That also keeps it off the real-time thread
entirely: no new command, no new atomic, nothing in `Player::fill`.

**The conversion is the part that is easy to get wrong.** A tick is song time, the offset is wall time,
and the tempo ratio is the exchange rate. At 1.25× speed, 40 ms of real latency is 50 ms of song time.
`shift_ticks` is a pure function in `km-display/src/lyrics.rs`, and its **early return at zero is
load-bearing**. At the shipped default the drawn tick is the published tick bit for bit. So a
rounding difference in the tick → µs → tick round trip cannot affect a machine that never sets this.

Two clamps, both about bounding inputs before they are multiplied:

- **`tempo_ratio` is clamped to `0.0..=MAX_TEMPO_RATIO`.** A float-to-int cast in Rust saturates, so an
  unbounded ratio makes the delta `i64::MAX`. Even with a `saturating_add`, `us_to_tick` multiplies by
  the ticks-per-quarter and overflows there instead. Clamping here keeps the fix out of `km-song`'s hot
  path. It is not theoretical: `Machine::new` seeds the live tempo from `settings.json` **without
  validating it**.
- **The floor is `0.0`, not `MIN_TEMPO_RATIO`.** Clamping *up* would invent movement where the caller
  said there was none.

**Three things it must not touch, each for its own reason.**

- The engine, because the audio is already right.
- `announce_lyric_line`, because the `lyric_line` event keeps the true tick. This calibrates *this
  machine's own television*, and a phone across the room has a different latency. Shifting the API to
  suit one screen would be a lie to every other client.
- The per-song reset in `start`, because transpose belongs to a performance while this calibrates a
  room.

**Clamped on write, not refused, and never gated on the song.** It follows the volume rather than
transpose. It is a calibration dial, nudged from a remote until the highlight lands right. A 400 in
the middle of that is noise. It also skips the `adjustable` check that makes the other
three answer 409 while a video plays. Those are about a MIDI song's notes, and this is about the
room's screen.

It rides `PUT /api/v1/settings` with no new route at all, which is the signal that this is
the right seam.

**On the appliance the number is 0**, judged *by ear* against audible sound. That is the load-bearing
word. The first pass happened while the box was silently playing into an unplugged sound card. It
judged how the wipe looked, not whether it agreed with anything. The 0 is a real result rather than a
skipped step: this room and this path need no compensation. It also gives the AV chain a baseline to
measure against once a receiver is in between.

## The queue overlay

**An overlay, not a screen.** A queue *screen* would have to replace the lyrics in order to show them.
And the moment anybody wants the queue is mid-song, when somebody asks whether they are next.

One row per waiting song, with the singer's name on the right in the accent color. At a party the
singer column is the one people read, because the question is "am I next", not "what is next". The
overlay cuts a long title by **character** rather than byte. That matters for a corpus this full of
accented Latin text, because slicing by byte would panic on a character boundary.

Three details each fix something a rendered frame showed:

- **Full-screen scrim at 82%, then the card twice.** Even a second pass of the ~78% panel left a lyric
  line clearly ghosting through. A large outlined glyph shows at 5% transmission far more than the
  arithmetic suggests. The scrim also makes the thing read as a layer rather than as text tangled up
  with the song.
- **`and N more` when the queue does not fit**, so the header count is never a lie by omission.
- **The count is omitted when the queue is empty**, because "0 songs waiting" beside "Nothing queued"
  says the same thing twice.

**Reachable from a television, which a `Q` binding was not.** A TV remote has a D-pad, an OK button and
a back button, and no letter keys. So `Keypad::playing` gained a `QUEUE` key. A six-key strip is wide
enough to reach the margins, and everything it can reach is along the bottom of the screen. The demo
line yields to it, and the standing connect panel goes for the same reason. The strip is a deliberate
interaction and both of those are ambient.

**Not built, deliberately:** removing or reordering from the overlay. It is informational, and a remote
is a better instrument for that than a D-pad.

## The queue count in the corner, and what a pill costs

`frame.queue.len()` again: the same number the overlay's header states, from the same field, so the
two cannot disagree. `draw` draws it rather than either screen function, in the slot the bank label
occupies. The stated reason is the same: it is a property of the machine, not of what is playing.

**SDL has no rounded primitive**, which the keypad already records as the reason its keys are squares
with a border. A pill cannot be squared off without becoming a different shape, so `fill_pill`
rasterizes it by hand. The straight middle is one `fill_rect`. Then each cap goes a row of pixels at a
time, evaluated at the row's centre against the analytic circle. That is `draw_qr`'s technique at a
finer grain. It is about 84 rectangles in one `fill_rects` call at 1080p, nothing beside the texture
upload one string costs.

The edge is hard, like every other fill here. If it ever needs softening, the answer is one
pre-rendered alpha texture blitted with `copy`.

**Three things the arithmetic got wrong and the contact sheet caught**, which is the part worth
keeping:

- **`Theme::glyph_box_px` is not the height of a line.** At 720p the small face reports a 26-pixel
  line where the glyph box computes 22.8. That function is right for *stacking rows*, which is what
  its other callers do. A box drawn **around** text has to be the size the text actually came out. So
  the pill takes its size from `measure_line`'s own `height`.
- **A pill is not a box.** Sized to exactly its line height, the digits touched it top and bottom.
  Worse, an ellipse *narrows* away from its centre line. So the corners of a `2` stood outside the
  curve while a bounding-box comparison said they fitted. `QUEUE_PILL_HEIGHT` is that air, and
  rendering found the number rather than reasoning.
- **The face is the small one because the row is `0.06h`.** That is 43 pixels at 720p, and the title
  face alone is a 36-pixel line. A pill around it could clear `SECOND_ROW_TOP` or have air, not both.

**And it bounded the badge run, which nothing had.** The key/tempo/melody run went straight to
`draw_text` with no `ellipsize` and no `fit_chars`. It was the only right-aligned text in the crate
without one, and it fitted a phone held in portrait by about forty pixels. Taking the corner away for
the pill made that reachable, so the cut landed with it.

It is the third instalment of the sequence the two sections below open on, and it failed the same way
both of those did. The contact sheet's badge fixtures are short strings on a 1920-wide frame. So a
sheet that exists to be judged by eye had nothing on it that could show the defect.

## The demo line, and why it does not share a row

`Frame::demo` is an `Option<&str>` that the caller resolves into a finished sentence. That is the seam
`soundfont_label` and `SongInfo::language` already use; `km-display` has no idea what a demo is and
needs none.

**`Frame::faults` is not one of them.** It is a count per area that this crate words itself out of
its own catalog. Quoting no package reason leaves its vocabulary a number and an enum. That is what
lets it live on this side of the seam and obey the machine's locale setting. The remaining three
cannot follow without `km-display` learning what a SoundFont is.

The demo line is drawn on the playing screen only and never timed out. It is hidden while the
transport strip is up, because both live along the bottom and the strip is centered over it.

**A row of its own is what `DEMO_LABEL_MIN_CHARS` buys.** A line sharing a row with an un-shortened
song title is capped at half the width. On a 2400-high screen, half a 1080-wide row at the small
face's size is **fifteen characters**. `DEMO · press S` is worse than saying nothing: it tells
somebody there is something to do and not what. That is the difference between this line and the bank
label one row from the top. The bank label shares its row happily, because a bank name ellipsized is
still a bank name.

The row is `DEMO_LABEL_TOP` (0.82). It is low on the screen because the line speaks to somebody who
has just walked into the room. That is also why it is never timed out. Two tests hold it:

- One asserts the line clears the lyric band above (about 0.66 with the shipped `Theme`) and the
  position bar below. It computes both from the theme and from `timeline_rect`, not from constants
  copied into the test.
- One asserts the whole instruction fits on every screen in `SCREENS`.

**The one thing beside it is on the right.** The standing connect panel takes the bottom-right corner
for the length of a demo song, which is the length of time this line is up. So the two share the
bottom of the screen, and the rule between them is that **the line wins**. The instruction may not be
shortened. `standing_panel_fits` asks what the line would come out at, and the panel is drawn only
where that clears `DEMO_LABEL_MIN_CHARS`. That is 25, which is `DEMO · QUEUE to sing next` exactly.

The four landscape screens in `SCREENS` leave it 84 to 112 characters; the portrait phone leaves 21
and gives the panel up. The line is capped to `standing_bottom_left_width` while the panel is up.

**The wording was then chosen against the measurement rather than the other way round.** Even with
the full row, the portrait bound fits 29 characters. So the television says
`DEMO · QUEUE to sing next` (25), and the remote carries the longer form. The remote has room, and it
is where the queueing happens. The half that cannot be worked out from anywhere else is that
*queueing* is the way through. A demo song has no queue entry to run out, and the next one starts the
instant this one ends.

**The four characters of headroom are what stop the wording growing.** `DEMO · QUEUE a song to sing`
(27) reads better and spends three of them. It spends them on the one screen where the connect panel
then stops fitting beside the line. Any rewording is measured against that before it is judged on
how it reads.

## The safe inset, and the two things that rest on it

`keypad::margin_x` and `margin_y` are the safe inset in pixels, `MARGIN_FRACTION` of each axis. They
are two functions rather than one taking both dimensions. The answers are different lengths on every
screen that is not square. A caller reaching for the wrong one gets a number that looks plausible.
`MARGIN_FRACTION` equals `Theme::margin`, and
`the_safe_inset_is_the_one_the_rest_of_the_screen_uses` holds the two together. It is one inset seen
from two modules, and only one of them has a theme to ask.

**`timeline_rect` is the only place the position bar's geometry lives**, and `timeline_thickness` the
only place its height does. The thickness has a two-pixel floor. So on a small window, a caller
measuring the bar as a fraction of the height would disagree with it. That leaves a gap or an
overhang, and it is why `position_reserve` derives from the thickness rather than being named beside it. Three
tests read `timeline_rect` instead of transcribing a number. They cover the bar inside the safe area,
the strip above it, and the bottom-left gap the bank label was refused.

**The strip reserves the bar's band the way the pad reserves the build number's.** `Keypad::playing`
takes a `reserve` exactly as `Keypad::idle` does, and the machine passes `position_reserve`. The two
arrangements are the same shape on the two screens. In both, the reserve comes out of the thing being
stood above rather than being a second constant either could contradict.

**The reserve is taken whether or not the bar is drawn**, so the strip holds its row while the bar
comes and goes. A reserve that followed the bar would move every button on the strip each time a song
crossed into the middle of itself. That is a worse answer than a band of empty pixels nobody can see.

## Two reasons not to draw the words, and why they are two fields

`Frame::picture` and `Frame::lyrics_hidden` both empty the lyric band and are deliberately not one
field. `picture` says the song brings its own words in its own image. So it suppresses the key and
tempo badges with the rows, because those name adjustments a video and an MP3+G song do not have.
`lyrics_hidden` says somebody withheld the words of a song that has them. A MIDI song still
transposes and still changes tempo with them withheld, so sharing the field would take away two
controls that work.

What they do share is the `(no lyrics in this file)` fallback, which both suppress. That sentence is
true of a file with no words, and false over a picture and over a withheld timeline alike.
`draws_lyric_rows` is where the two terms meet, so neither the rows nor the fallback can come to
disagree about which frames draw words.

`badge_run` is a function of the frame and nothing else, so a test can read what the run says without
a canvas. The drawing beside it is a join, a cut to fit and one `draw_text`. **Order is
content there.** The run is right-aligned and ellipsized against the queue pill, so a screen too
narrow for all four loses the last. The words badge goes first, because it is the one that explains
an empty middle of the screen.

## When the bar is drawn, and the furniture over a picture

**`Frame::show_position` is the whole of the rule and the caller's fact**, so `km-display` composes
nothing. `draw_progress` reads that one boolean, and `km-display` holds no opinion about when a bar is
wanted. `position_visible` is where the three terms meet:

- `within_position_window` is the ambient one.
- `position_wanted` answers somebody standing at the machine.
- `playing && !advancing` holds the bar up for as long as a song is stopped, since no window can read
  a position that has stopped advancing.

**The window is the caller's because only the loop can tell a fresh position from the previous
song's.** `Snapshot` takes `now_playing` from the locked state. It takes `position_ms` from an engine
atomic that the audio callback alone writes, and `publish_stopped` leaves that atomic where it was on
purpose. So a frame between the load and the device's first report carries one song's length beside
another song's position.

`position_ms < window` hides the bar as a song begins. `duration_ms.saturating_sub(position_ms)` calls
that beginning an ending whenever the song before was the longer.
`a_song_change_shows_the_bar_before_the_audio_reports` pins both readings and the deadline
that covers them.

`position_until` is a fifth deadline of the loop's usual shape, beside `CONNECT_GREETING`,
`strip_until`, the flash's and the keypad line's. Four places raise it: any bound key, the D-pad, a
mouse or finger press, and the song-change block. That block already detects a song change for the
window title.

**That fourth raise is load-bearing** rather than a courtesy. It draws the bar through the frames the
paragraph above describes. Six seconds covers them, because a queue advance costs one device period.
A device reopening after an idle gap has the four seconds of `OPEN_RETRIES` to get there. One
`Instant::now()` serves `position_wanted` and `strip_visible`, so a frame cannot answer two questions
from two clocks.

`position_visible` reads the deadline rather than `keypad_visible`, and that is the part worth
protecting. `a_machine_with_no_transport_strip_keeps_a_picture_songs_bar` drives both real functions.
A test spelling the condition itself would go on passing after somebody simplified the loop to read
the master switch.

**A length of zero is refused the unasked bar and granted the asked one.** A bar with no length cannot
move. So `within_position_window` requires `duration_ms > 0`, and `position_wanted` does not. Then
`draw_progress` draws the track without a fill. That is the answer a key press earns, and not one
worth a row of a song's words unprompted.

**Yielding is not enough for a line where the words are.** Six tenths of an opacity keeps furniture
readable over a picture and keeps the picture readable under it, which is the whole trade. But a
video song's own words are burned into its frames near the bottom. A line drawn there is two
sentences in one place, whatever the opacity of either.

`NEXT_UP_TOP` is the third row of the top-left column (0.17) for that reason. The column is the machine's own for three rows, and no song puts words
in it. `next_up_sits_below_the_artist_line_and_above_the_lyrics` holds both ends against the theme.

Three things move with it:

- The bank label keeps the top right. The argument is that the corner is where this screen puts
  machine state, which is `Which bank is playing`'s own and needs no second one.
- `DEVELOPER_LABEL_TOP` is the same row seen from the other side. So `NEXT_UP_MARKED_WIDTH` is
  everything that marker's cap leaves, and the two share a band rather than a column.
- The frame panel at `PERFORMANCE_TOP` (0.16) runs down over the line while it is up. That is the
  precedence the strip already has over the demo line: a panel somebody switched on outranks an
  ambient row.

**The furniture stands at full strength over a picture song.** Every piece of it passes through
`draw_playing`'s three style closures, and none of them asks for less than the theme names. Measured
over a ground of `0xF0`, the `artist · language` row comes back at the theme's own `text_dim`. Its
ring comes back at the theme's own `lyric_outline`: 149 and 11 against a picture at 240.
`examples/preview.rs` holds those three numbers, in the cell that renders the furniture over a pale
picture.

**A glyph yields as one mark, and a rectangle yields through its color.** `theme::fade` scales the
alpha a color already carries, so `theme.panel` at `0xC8` composes rather than jumping; that is the
position bar's track. `TextStyle::faded` carries a fraction on the style instead, and `draw_text`
applies it to the finished texture with `set_alpha_mod`. Nothing on the playing screen asks for a
fraction. The mechanism is here because the wrong way to fade text is the reachable one: see below.

**A string is composed before it is uploaded**, so the ring cannot show through the fill. `TextCache`
holds one texture per string per ring: the eight dark copies and the glyphs over them.
`compose_outlined` composites them at full strength. An opacity is therefore a modulation of a
finished image.

Folded into the two colors instead, the ring's eight blits compound where they overlap. The dark
under a stem then stands at all but opaque, and translucent glyphs blend with that rather than
covering it. At six tenths over a ground of `0xF0`, the `artist · language` row composes to a
brightness of 94. Six tenths of its own color gives 184. That is what a fraction costs when it is
applied a layer at a time. It is why `faded` is a field on the style rather than a call on two colors.

Two facts about the composition are worth carrying, because each is a wrong answer that looks right.
Blending onto a transparent surface through SDL's ordinary mode leaves color premultiplied by alpha
beside a straight alpha. So an antialiased edge uploaded from it arrives dark over a pale picture.
`compose_outlined` writes Porter-Duff `over` out and divides back to straight alpha once per pixel.
That is also what makes `set_alpha_mod` the whole of the fade. A premultiplied composite would need
`set_color_mod` set with it, or a faded string would glow rather than thin.

**Nine layers, one pass.** The accumulation carries premultiplied color across the nine and divides
at the end. So a string costs one pass over its result rather than one per layer. The largest run on
screen is a lyric line, and it is composed again every time the words change.

**Text blits from a rounded origin.** The rows of this screen are fractions of its height. A 1:1 blit
landing on a half-pixel is resampled across two rows of texels. `TITLE_ROW_TOP` at 0.05 of 1080 is
whole; `SECOND_ROW_TOP` at 0.11 and `NEXT_UP_TOP` at 0.17 are not. That is why those two rows read
softer than the title above them. `draw_text` rounds the origin and `draw_wiped_line` rounds before it
splits, so its two passes agree about where the line starts.

**The wipe's extent keeps its fraction**, which is what carries the highlight smoothly across a
syllable. The scale mode stays linear for the same reason: `Nearest` would snap that one edge to a
whole pixel.

**The ring resolves against the face, inside `draw_text`.** `Theme::outline` is a fraction of a
face's own size, and `TextStyle` carries it rather than a pixel count. A style is built before a face
is chosen: `draw_playing`'s three closures serve the title in `fonts.text` and the two rows under it
in `fonts.small`. `draw_text` asks the font for its size through `TTF_GetFontSize`, and
`Theme::outline_for` rounds it. So a caller cannot hand one face and the ring of another. `Layout`
therefore carries no thickness, because there is no single right one for a frame.

**The cache keys on the ring and not on the opacity.** A faded string is the same pixels modulated on
their way to the screen. So a picture song and the machine's own screen share an entry; a string and
the same string ringed more heavily are two. A ringed string holds one entry where the fill and the
outline were two. The composition is larger than a glyph run by twice the ring on each axis. So
`BUDGET_PX` covers more strings rather than fewer.

**`render_to_image` takes a `Backdrop`**: the image, the scrim and the shape. They are one fact rather
than three loose arguments, and they differ in nothing else between a wallpaper and a song's own
picture. It exists so `examples/preview.rs` can render a frame over a letterboxed picture.
`cdg_test_picture` builds a 288x192 field on the 6x12 tile grid with words on rows 0, 1, 14 and 15,
handed over at 4:3. Cells `25` and `25b` are the bar waiting and the bar wanted. Nothing in this
repository could draw that arrangement before, so every judgement about the opacity happens there
rather than by arithmetic.

## A lyric line that will not fit

**The display had no horizontal fitting for lyrics at all.** A line is centered on half the width with
its true measured width. So a centered alignment gives a negative left edge for anything wider than
the screen, and SDL clips both ends. Crate-wide, the only comparison of a measured width against the
layout width was the idle title's. This is not specific to any one corpus; the real one holds lines
of 1,667 characters.

`Fonts::fit_lyric` measures against a ladder of fallback sizes. It returns the largest face that fits
the width less its margins, with its metrics. At most four measuring passes for a wide line and one
for an ordinary one; nothing is opened and no texture is made.

**A line whose word gaps are `km_song`'s `SYLLABLE_DIVIDER` measures a few percent narrower.** So it
can take a larger face, with no display code involved. The narrowing is in the parsed text, which is
why nothing here has to know about it. The face must carry the glyph, since a missing one draws as
`.notdef`, a box mid-lyric. The lyric faces are the system's rather than bundled.

Five details that are easy to get wrong, and one that already was:

- **The ladder is loaded, not computed.** A face must exist to measure with, and a draw cannot open
  fonts. So the sizes are height-derived like every other face and only the *choice* is about width.
- **The size list must name them**, or a height change leaves the ladder at the old screen's size. It
  still takes no width, and should not: the choice is made per line, per frame.
- **Each row chooses independently**, so a long line does not shrink a short one.
- **The same face measures and draws.** The wipe offset is an offset into the metrics, so a mismatch
  puts the highlight inside the wrong glyph.
- **The upcoming row had never been measured at all.** It went straight to `draw_text`, which asks the
  rendered surface how wide it came out, and so could not have noticed. **That is the one this list
  exists for.**
- **The fitting decision is a pure function over widths**, because no test in this crate may open a
  font.

**A contact sheet that renders no long lyric cannot show a defect in long lyrics.** The fixture's
lines are 27 and 25 characters. **So a sheet that exists to be judged by eye could not show the one
defect the eye would catch instantly.** It carries a 56-character line and an 85-character one.

## A line-timed song, drawn a line at a time

**`LyricView::frame` decides the mode per frame from `LyricTimeline::granularity`.** A line-level
timeline reports its current line as the last syllable at full progress once the line has started.
`draw_wiped_line` then draws it whole in the sung color, so the renderer needed no mode of its own.
The cost is one pass over the lines per frame, which is small beside drawing them.

**Two fields on `VisibleLine` carry the rest, and both stay neutral for a syllable-timed song.**
`opacity` fades the current line once its singing is over, before a long gap. `cue` is the fill of
the lead-in bar over a line that starts after one. `brightened` moves the upcoming row from the
upcoming color to the pending one, through `draw::mix`. `draw_lead_in_cue` draws that bar above the words,
inside the row, so it stays in the band the wallpaper pack measures.

**The thresholds are beats, scaled by `for_ticks_per_quarter`**, as the lead-in is. A millisecond
timeline's beat is half a second, so the hold is four seconds and the cue two. `CUE_BEATS` is
asserted no longer than `LEAD_IN_BEATS` at compile time, because a cue on a line not yet shown
fills under nothing.

**`sung_until` trusts an end the file gave.** The file bounded a line whose end is before the next
start: a blank LRC line, or the last line's hold. Only a line that runs into the next start
takes the hold instead. The faded line stays the frame's current line, so `current()` and the
page logic see nothing new.

## The keypad line, which was the other one

**The keypad line had the same fault, and it went unfixed for another release.** The sentence
"Crate-wide, the only comparison of a measured width against the layout width was the idle title's"
was true of it as well.

The keypad prompt draws three things in `fonts.lyric`, centered on its true width. They are the
digits, the `Enter a song number` hint, *and every message the machine sends to somebody at the
keyboard*. That is the same arrangement, the same negative left edge and the same clipping at both
ends. It reached a real screen as `no melody channel was detected for this song`. That is 44
characters, about 1,250px in a 61px face, against a 1,280px window with 64px margins.

`fit_prompt` walks the same `Fonts::fit_lyric` ladder, so `10234` and `no song 9999999` are drawn
exactly as they were. Then it does the one thing a lyric row may not: it **wraps**. The prompt goes
into the small face, at most two lines, cut with the notice's `…`.

The row beneath a lyric holds the upcoming
line. The row beneath the prompt holds nothing, and the dialled title that could sit there is never
drawn beside a message. `wrap_prompt` is split out for the reason `first_fitting` is. No test in this
crate may open a font, so the part that decides what the words do sits where a test can reach it.

The playing screen's copy of the line needed the panel to grow, not just the text to fit. Cut to the
width of the `0.3w` box it lives in, the same sentence came out as `no melody channel was det…`. It
now takes the wider box that a dialled title already asks for. The same entry can never want both at
once: `show_message` clears the digits, and a preview only resolves for digits still on screen.

**And the review gap is the same review gap.** The paragraph above says the contact sheet had never
rendered a long lyric. It had never rendered a long *message* either. `no song 9999999`, fifteen
characters, was the only one on the sheet. The long-string cases it did carry all went down the
ellipsized preview path. It now carries the 44-character refusal on both screens.

**Its clock is in `NumberEntry`, not in the loop.** The three deadlines beside it are `Instant`s
compared while the frame is built: `CONNECT_GREETING`, `strip_until`, `Flashed::until`. `draw.rs`
holds no state, and the machine owns the clock.

This one is a `Duration` that
`tick(delta)` counts down, from the same frame delta the wallpaper schedule already takes. The state
it belongs to is `NumberEntry`, and this crate's tests may not sleep. Without it, only key presses
ever cleared a message. So a refusal about a song outlived the song, the queue and the return to the
idle screen.

## The three keys past the strip: F10, F11 and F12

All three are bound in the display's table. The **machine's event loop intercepts them** rather than
the shared handler, because each needs something the handler is not given:

- the paths and the loop-local flash for `F10`;
- the API state for `F11`;
- the loop's own meter for `F12`.

That is the same reason the loop intercepts Escape, the fullscreen toggle and the always-on-top
toggle. The handler still gets an explicit arm for all three, so adding an action stays a compile
error. That is exactly how `ToggleAlwaysOnTop` and `ToggleStripPin` announced themselves.

The modified row is all three, each paired with its bare key. `F10` shows the folder and `Ctrl+F10`
reads it again. `F12` draws the meter, and `Ctrl+F12` stops the strip vanishing from under whoever is
reading it. `Ctrl+F11` is the odd one: it is not a second action but a second spelling of the same
one. It is bound because macOS's window server takes bare `F11` for *Show Desktop* before an
application is offered the press. It is the one arm in the `ctrl` branch that repeats a bare
binding, and the test asserting the pair holds it to that.

`F10` was `F12` until the other two arrived; the decision records why the argument that put it there
is the argument that moved it.

Things about `F10` and `F11` that are not obvious, most of which apply to both:

- **Neither is a transport-strip entry**, and both bindings sit *above* the fall-through that reads
  that table. The strip is drawn over a playing song. A `SONGS` button among `PAUSE` and `KEY +` would
  be the one control there that is not about the song. A test asserts both halves: that each key
  means what it should, **and that the strip lookup still returns nothing for any of the three**.
- **That second half stopped being hypothetical when `F10` was taken.** It is the very next key after
  the strip's nine. So a tenth `TRANSPORT_COMMANDS` entry would now be a button drawn on the screen
  that no key can press. The test therefore asserts the table's **length** as well, with a message
  saying exactly that.
- **All three are in the auto-repeat filter.** The original members are there because a held key
  undoes itself. **`F10` is there because a held key opens a file-manager window per repeat, and this
  process can close none of them.** `F11` is there for the same reason with browser tabs. `F12` is
  there because a held toggle lands on whichever parity it stopped on. That is often the one it
  started from, which looks exactly like the key not working.
- **The opener runs on a thread of its own.** Both `xdg-open` consulting a desktop and `cmd /c start`
  spawning a shell are long enough to drop frames mid-song. A browser cold-starting is seconds of it.
  Its report comes back through a channel that lives for the whole loop, **because a receiver has to
  outlive the frame the press happened in**. **The two keys share one channel**, since
  both can only ever report a failure and a second `try_recv` would buy no property.
- **Only failures go down it.** A folder or a browser window that opened is standing in front of the
  machine. A band across the lyrics saying so would be the weaker of the two statements. The loop logs
  both outcomes, on the standing rule that a press producing nothing must not also be silent.
- **`F11` refuses in two different ways, before starting any thread.** `api.serve_remote` false means
  `/` is the API's landing page. So the key names the setting rather than opening an index of
  endpoints. A server that never bound carries the connect panel's own words for the port conflict.
  `remote_url` is split from `show_remote`, so tests assert both refusals without launching a browser.
  That is the same split `folder_to_show` has, and for the reason `km_osopen::command_for` has it.
- **`F11`'s URL is loopback, and that is not the address on the television.** The panel answers
  *where is the machine* for a phone. `F11` answers *how does this box reach its own server*. The two
  differ in both directions. `connect::own_url` is three cases:
  - An unspecified bind reaches itself at `127.0.0.1`.
  - A specific address is opened where it is, because nothing is listening on loopback.
  - A machine with no network at all still opens. `ConnectInfo::primary_url` would refuse that, since
    its `urls` are empty exactly then.
- **The icon in the bar is the panel's question, not `F11`'s**, so `connect::reachable_url` sits
  beside `own_url` rather than replacing it. It is `primary_url` with `own_url` beneath it. Somebody
  about to type an address into a phone reads a streaming machine's menu. The fallback is there so a
  machine with no network names something rather than nothing. Which surface asks which question is
  [`The machine's icon names the address a phone can reach`](../decisions/interface.md#the-machines-icon-names-the-address-a-phone-can-reach).

### The line on the panel is a sixth `cfg!` const, narrower than the one that gates the press

`BROWSER_KEY_HINT` is `windows` or `macos`, where `WEB_BROWSER` is everything but Android and iOS. The
two sit beside each other in the same family, and that is the point: they answer different questions.
One asks whether a press is worth attempting, and on Linux it is. A desktop opens the page, and a box
without `xdg-open` comes back with the reason on screen. The other asks whether to make the offer
before anybody presses anything. A constant that cannot tell a bare TTY from a Linux desktop should
not promise the appliance a browser.

**It is the one in the family that is not a `bool`**, because it answers three ways: `F11` on
Windows, `Ctrl+F11` on macOS, nothing elsewhere. The macOS window server takes the bare key for
*Show Desktop*, so the offer there has to name the spelling that arrives. Both are bound in
`input.rs` on every platform; this decides only which one is drawn.

**The display crate is told rather than asked**, which is `factory_pin`'s arrangement exactly.
`ConnectInfo::browser_key` is an `Option<BrowserKey>` that `connect::to_display` fills in on the way
past. So the crate that draws for five operating systems holds no opinion about any of them.
`ConnectPanel::hint` is what comes out the other side, worded from the catalog beside the panel that
draws it. It is two message ids for one sentence, differing in the key they name.

**What it cost the geometry is nothing, and that was not free.** `HINT_MAX_LINES` comes out of
`DETAIL_MAX_LINES` rather than beside it. All three panel sizes are anchored under the lyric band, and
the clearance is thinner than it looks. `2560x1080` leaves the standing card **nine pixels** against a
row of small text costing forty-four. Shrinking the code to the overlay's buys twenty-seven of them,
and that size is the floor the card's whole shape rests on.

So the standing card carries no hint at all, and the other two spend the detail's rows. The detail
wraps first, the hint takes what is left, and where nothing is left it is not drawn.
`the_key_hint_shares_the_panels_rows_rather_than_growing_its_box` keeps that measurement rather than
the conclusion. **The five characters `Ctrl+` costs buy nothing back.** Both spellings are measured
across every screen and both languages. The portrait phone is the only screen either loses, the same
one it lost before.

**Running it found one thing no reasoning had, and it is a Windows fact worth keeping.** `cmd /c start
"" <path that does not exist>` does **not** fail: it puts up a modal message box and does not return
until somebody dismisses it. So the one plausible Windows failure would never have reached the screen
at all: no exit code, no flash, and a parked thread. The folder is made before being handed over.

## F12 draws what the meter measures

The meter is the same `FrameMeter` `--frame-stats` has always used. The change is which of its two
jobs the flag controls. **The flag decides only whether the meter also writes a line**, not whether the
meter exists. The meter is built when either the flag or the key asks. Pressing `F12` on a machine
started without the flag builds one with `logging: false` and drops it again on the way out. So a
machine nobody is diagnosing goes back to doing no arithmetic per frame.

**Whether the panel is up is `Machine`'s and not the loop's.** It is an `AtomicBool` that the key and
three pages all move: `/admin/`, `km-admin` and `/dev/`, through `GET /api/v1/performance` and
`PUT /api/v1/admin/performance`. In memory only: see
[`F12 is one way in, not the only one`](../decisions/interface.md#f12-is-one-way-in-not-the-only-one)
for why writing it down would make a diagnostic a setting.

**The meter is therefore built and dropped once per frame rather than on the keypress.** That is the
one thing this arrangement requires. The key is not the only thing that can change the answer, so
reconciling on the *event* would miss a page. One atomic load per frame, which is
what `foreground` already costs.

`record` computes a `km_display::FrameStats` once when a window closes. It logs from **that** rather
than from the accumulators, and carries it across the reset in a `last` field. That field sits beside
`decode`, which is carried for its own reason. Two consequences worth stating:

- **The panel and the log cannot disagree.** They are one struct, built once, over one second.
- **`frames: 0` is a state and not a missing value.** It is what the panel draws as `measuring…`. That
  is why pressing the key produces something immediately rather than a blank box for up to a second.
  A window that closes on no frames at all keeps the previous one rather than blinking back.

`FrameStats` lives in `km-display` rather than here, with `strained()`, `decoder_complained()` and
`frame_is_tight()` on it. The machine measures, and the display crate decides what is worth coloring.
That is the same seam `lyrics` sits on, and it lets the thresholds be unit-tested with no screen.

The two conditions are independent and fail in opposite directions. A decoder that has stopped moves
the counters and leaves the timings at a perfect 60 fps. That is the fault actually seen on the
appliance. A machine that cannot build a frame in time moves the timings and leaves the counters at
zero on every MIDI song.

### The song block, and where its two halves come from

`SongStats` sits beside `FrameStats` in `km-display`, on the same seam and for the same reason. The
machine decides, and the display crate decides what is worth colouring. Its predicates are
`worth_attention()`, `damaged()`, `fixed()` and `gain_is_steep()`, all unit-tested with no screen.
`damaged()` is `truncated_tracks` or `missing_tracks` and deliberately not `repaired_notes`. That is
the cut `warn_if_damaged` makes in the log, so the panel and the log cannot disagree about whether a
file is damaged. It is the song half of the property the two frame reports already have.

**The block is assembled from two places, and the split follows what each fact is about.** The gain,
its derivation and the two channel counts come from `Machine::song_levelling`, because `Loaded` is
private to `machine.rs`. The flavor, the dialect, the track count and the three damage counts come
from the loop's `Arc<Song>`. The loop already holds it at the top of the frame, so that costs nothing. That
`Option` also says the song is a MIDI one, because `current_song` is `None` for the other two kinds.
So no song kind travels to `km-display` to be asked about.

**The position row is the one fact not in `SongStats`.** `draw_performance` reads it from
`Frame::position_ms` and `Frame::duration_ms`, the smoothed pair the position bar draws, because
`SongStats` is taken once when a song starts. It carries that pair's one known lag: for the frames
before the audio device reports, it shows the song before's position.

**The gain is recorded where it is worked out.** `start` sends one `SetSongGain` and the engine
replays that value into every stream it rebuilds, so a bank switch mid-song leaves it in force.
Anything recomputing against the bank sounding now would draw a number the audio path is not using.
The number and the word beside it come out of one expression, so they cannot disagree about what
happened to the song.

**The public API snapshot is untouched.** That struct is the remote's wire shape, and this is a
diagnostic drawn on the machine's own screen. `current_song` and `current_cdg` are the precedent for
a display-thread accessor the API knows nothing about. The lock is taken only while the panel is up.

**Both blocks count their own rows**, which is what lets a test bound the panel's height without a
font. The count varies with the song. So how far down the panel reaches is not a number anybody can
hold in their head while adding a row. What it must clear is the demo line's own row; the lyric
ladder below `NO_LYRICS_TOP` is a trade this diagnostic already took. The width bound holds with
words in the panel, because every value is right-aligned at the panel's right edge and grows
leftwards. So overflow reaches into the padding rather than toward the developer marker.

**Nothing about the appliance is special-cased**, which was considered and rejected. A Debian box with
no desktop has no `xdg-open`, so the press comes back with a not-found that goes on the screen. The
alternative was a runtime probe of SDL's video driver, and it was not taken. **A constant which
cannot tell a bare TTY from a Linux desktop should not be pretending to.** Refusing a
key the machine could actually have honored is the worse of the two failures.
