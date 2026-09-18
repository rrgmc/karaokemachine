# The desktop shells

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## The icon in the bar, and the event loop

`km-tray` is the icon in the Windows notification area and the macOS menu bar, for the four programs
that start a web server and then get out of the way: `km-package-builder`, `km-remote`, `km-admin`
and the karaoke machine on a streaming run. A library, two thin binaries, `tao` owning the main
thread, and the runtime built by hand and handed to whichever shape takes over.

**Three of the four serve one page and the machine serves three**, which is the whole of what
`Pages` is for: it says which of the two menus a run is asking for, and `items_for` turns that into
the entries. The crate names the entries and never holds a second address — the shell knows where
each one points, which is what keeps this a crate about an icon.

**The event loop is entered whether or not there will be a window, and the window is a flag inside
it.** An icon needs a platform event loop exactly as a window does — **and the runs that most need an
icon are the ones with no window**, since `--browser`, `--lan` and the webview fallback all leave a
server listening with nothing on screen.

Two things follow that are easy to get wrong:

- **The loop has to end when the server does.** A windowless run cannot sit on the server's
  `JoinHandle`, and a loop owning the main thread would otherwise spin in front of a dead server for
  ever, so a watcher task sends a user event. That handle must not be dropped: detaching the task is
  harmless while closing a window is the only way out of a run, and not harmless once runs have no
  window to close.
- **The webview fallback is an ordinary run of the loop**, not a shape of its own. Parking the thread
  in a sleep loop there leaves Ctrl-C shutting the server down and the process alive, tokio's handler
  having suppressed the default terminate.

**The crate takes no `tao`.** It hands out a command through a plain `impl Fn(Command) + Send + Sync`
and each shell wires that to its own event-loop proxy. Forwarding rather than acting matters on its
own: the menu and tray handlers run on the platform's event thread, **which is not a place to close a
window from**.

Seven platform details that are not obvious:

- **The icon is created on `StartCause::Init`, not before `run`.** `tray-icon` requires this on macOS
  — an icon made before the loop is running misbehaves against full-screen applications — and Windows
  does not care, so one arm serves both. It is dropped in `LoopDestroyed`, which takes the icon out of
  the bar rather than leaving a ghost.
- **Windows reads the picture out of the executable's own resources**, the same route the title bar
  takes: no decoder, no second copy. **macOS decodes**, because the remote ships there as a bare
  executable with no bundle to read it back out of. That is the crate's only `image` dependency, under
  a target `cfg` table.
- **The decode fixes the height and lets the width follow.** A menu bar gives every item the same
  height and as much width as it asks for, so height is the one dimension a mark has to match;
  resizing to a square would scale the two axes by different amounts and draw anything that is not
  square stretched. Three of the four marks are square tiles and cannot tell the difference; the
  machine's streaming mark is a pair of letters and does.
- **`Spec::icon_is_template` says the picture is a silhouette macOS may color**, which is what every
  glyph beside it in that bar is. Only the machine's streaming mark passes `true`; the three tiles
  cannot, a tile's silhouette being the tile. It is a property of the picture rather than of the
  platform, so it is a field rather than a `cfg`. Windows has no such idea and ignores it.
- **`tray-icon` is declared per target, naming Windows and macOS rather than excluding Linux.** The
  crate is a workspace member, so a workspace build compiles it everywhere, and the negative spelling
  would ask for `libayatana-appindicator` on the BSDs — a platform nothing here builds for and nobody
  would notice until it failed. `build` still exists everywhere and answers with an error, so no
  caller needs a `#[cfg]`.
- **The menu's shape is a pure function** of the `Spec` — whether there is a window, and which pages
  the run serves — and the id mapping a pure function of a `&str`. That puts the rules worth pinning
  inside tests that run on a machine with no icon bar, which is every machine CI uses: `Show` absent
  rather than grayed when there is no window, an entry absent for a page the run does not serve,
  every id round-tripping to its command, every command offered by some menu, and a left click asking
  for something the menu it belongs to contains.
- **A left click is decided beside the menu rather than inside the builder**, because the two can
  disagree and the disagreement is silent: a click that sends a command the menu never offers reaches
  a shell with no reason to handle it, and nothing happens at all. That is not hypothetical — the
  click fell back to `Open in browser` whenever there was no window, which is exactly the entry the
  machine's menu does not have.

### The macOS application menu

**`km-tray` owns it too, and the crate's name is the only thing that makes that surprising.** It is
one function beside the icon's — `install_app_menu` — for one reason: `muda`'s
`MenuEvent::set_event_handler` is a process-wide slot whose *first* writer wins and whose later ones
are discarded, not merged. Two menus therefore have to be one registration, so both are routed
through a single forwarder held in a `OnceLock` and the app menu's Quit carries the tray's own id.
The installed handler captures nothing and looks that forwarder up, which is what stops the two
"once"s being able to disagree about which call won.

**The two calls are separate rather than one folded into the other**, because they are two platform
objects wanted separately and failing separately: a `Spec` describes an icon and a menu bar uses none
of it, the icon is something a run can be asked to go without, and one call answering for both could
report only one failure where a caller needs to know which of the two it lost.

**A run with no window gets the menu too.** The loop is entered whether or not there will be a window,
so a `--browser` or `--lan` run — a server listening with nothing on screen — is still an application
in the Dock, and ⌘Q stops it through the same shutdown. Measured on macOS: `km-remote --browser`
shows the bar and quits on ⌘Q with `stopping` in
the log, which is the graceful path rather than a `terminate:`.

**Quit is a `MenuItem` and never `PredefinedMenuItem::quit`.** The predefined one is `terminate:`,
which reaches `applicationWillTerminate` — so `tao` does still emit `LoopDestroyed` and the teardown
does run — but it never passes through the handler, which is where each shell asks its *server* to
stop. The visible symptom would be the remote sitting out its shutdown grace waiting for an outcome
nothing started.

**It is installed on the way into the loop, where the icon is installed from inside it.** The icon's
timing is `tray-icon`'s rule about status items and full-screen applications; a menu bar has no such
rule, needing only the `NSApplication` that `EventLoopBuilder::build` has already arranged. Nothing
removes it on the way out: `tao::run` calls `process::exit`, so the menu bar goes with the process.

### A console twin says it wants its console

**`km-console` takes a `Console::{NotWanted, Wanted}` and the executable says which it is.** Deciding
on the console's **process count** alone is wrong for half the pairs: one process means this process
is alone in a console, which is what a double-click arranges — right for an executable handed a
console it never asked for, and **exactly backwards for the one whose reason to exist is to be read**,
which frees the console and closes the window a moment after it opened.

**Nothing observable separates the two.** Both halves of a pair can be double-clicked, both are handed
the same console, and the subsystem is a flag in a PE header the process would have to read about
itself.

The rule lives *outside* the `cfg(windows)` module so a test can pin it on every platform: two of the
three CI platforms could not otherwise check it.

A double-clicked twin has somewhere to talk, so it prints its address instead of opening a browser
tab.

## Where a window opens

`km-webshell` holds the one thing the three shells' `desktop.rs` files agreed about byte for byte:
the arithmetic that clamps a wanted window to the screen and centres it on the right monitor. It has
no `run`, no `WebView` and no `wry` — the crate that decides where a window opens does not drag a
browser engine in with it.

**`tao` is declared per target there for the same reason `tray-icon` is in `km-tray`, and the cost of
learning that twice was 113 commits.** A plain `[dependencies]` entry made `cargo clippy --workspace`
and `cargo test --workspace` fail on Linux inside `pango-sys` — `tao`'s Linux backend is gtk3, which
`tools/platform/linux/apt-deps.sh` installs none of — before either reached a line of this
repository's own code. **A feature would not have helped**: the three shells already gate the crate
behind their own `desktop`, and `--workspace` builds a member whatever its dependents asked for.

**What is left on Linux is the part worth having there.** `fit` and `centre` speak no `tao`, so they
and their tests compile and run on every platform, which is why the crate stays a workspace member
rather than being excluded from it. Only `opening_geometry` is `cfg`-ed out, and unlike `km-tray`'s
`build` it gets no stub arm on the other platforms: it takes a `tao` type, so there is no signature
to stub without the dependency being avoided.

**Only a Linux `--workspace` build sees it**: CI's `linux` job and `tools/platform/linux/check.sh`.
`cargo km-test` on Windows compiles `tao` against WebView2's own backend and is happy.

## The webview's own profile

**Both windowed programs name their webview's user data folder.** A bare builder leaves WebView2 to
choose, and its choice is `<exe name>.WebView2` **beside the executable** — worst where both programs
sit together in a staged folder, and worse again where that folder is not writable.

**Only WebView2 reads the field, and the code is shaped to say so.** The data-directory option is
consumed in exactly two places in `wry` — the GTK backend, which this project never builds, and the
WebView2 one. The macOS implementation is a unit struct that discards the path, because WKWebView
keeps its state in the application's own container. So the helper is a `cfg(windows)` /
`cfg(not(windows))` pair returning `None` off Windows: **a path returned there would create a
directory nothing would ever open.**

**The directory is created here rather than left to WebView2**, so a failure becomes this function's
`None` — a wry default, and a window — instead of an environment that will not build and a run that
falls back to a browser it did not need. `None` never stops the window opening.

**The context is a local and is dropped when the function returns.** That looks wrong against wry's
warning that dropping one loses custom-protocol actions on macOS, and is not: the builder unifies the
context borrow with the window's and returns a webview carrying no lifetime, so both end there — and
neither program registers a custom protocol.

**Two crates, two copies, deliberately.** These files are acknowledged verbatim twins and share only
the tray crate; fifteen lines apiece is the cheaper side of that trade. **The thing that must not
drift is the directory qualifier**, and each file uses the one its own crate already uses elsewhere.

The tests decline to assert `Some`, because a platform that will not name a home directory is a
legitimate `None`. **What they pin is what would regress silently**: a relative path, or one under the
executable, would both still work and would both put the profile back where this exists to move it
from.

**A named profile keeps an HTTP cache, and that is what makes a landing redirect a promise.** The
cache outlives the run that filled it, so what the server answered at a URL the shell opened is
followed on the next launch rather than asked again — and a *permanent* redirect is the one answer a
browser is entitled to keep for good. A shell therefore opens the page it exists to show rather than
an origin that redirects to it, and a landing redirect that is served at all is temporary. Where
those two rules live is `Bound::front_door` and the `/` route in `km-admin`'s `server.rs`, and the
failure they prevent is a window that opens on a path the build no longer mounts.
