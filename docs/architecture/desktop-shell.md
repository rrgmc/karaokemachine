# The desktop shells

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## The icon in the bar, and the event loop

`km-tray` is the icon in the Windows notification area and the macOS menu bar. It serves the four
programs that start a web server and then get out of the way. They are `km-package-builder`,
`km-remote`, `km-admin` and the karaoke machine on a streaming run. The crate is a library and two thin binaries.
`tao` owns the main thread, and the shell builds the runtime by hand and hands it to whichever shape
takes over.

**Three of the four serve one page and the machine serves three**, and that is the whole of what
`Pages` is for. It says which of the two menus a run is asking for, and `items_for` turns that into
the entries. The crate names the entries and never holds a second address. The shell knows where each
one points, and that keeps this a crate about an icon.

**The event loop is entered whether or not there will be a window, and the window is a flag inside
it.** An icon needs a platform event loop exactly as a window does. **The runs that most need an icon
are the ones with no window.** `--browser`, `--lan` and the webview fallback all leave a server
listening with nothing on screen.

Two things follow that are easy to get wrong:

- **The loop has to end when the server does.** A windowless run cannot sit on the server's
  `JoinHandle`. A loop owning the main thread would otherwise spin in front of a dead server for ever,
  so a watcher task sends a user event. That handle must not be dropped. Detaching the task is
  harmless while closing a window is the only way out of a run. It is not harmless once runs have no
  window to close.
- **The webview fallback is an ordinary run of the loop**, not a shape of its own. Parking the thread
  in a sleep loop there leaves Ctrl-C shutting the server down and the process alive, because tokio's
  handler suppresses the default terminate.

**The crate takes no `tao`.** It hands out a command through a plain `impl Fn(Command) + Send + Sync`,
and each shell wires that to its own event-loop proxy. Forwarding rather than acting matters on its
own. The menu and tray handlers run on the platform's event thread, **which is not a place to close a
window from**.

Seven platform details that are not obvious:

- **The icon is created on `StartCause::Init`, not before `run`.** `tray-icon` requires this on macOS,
  because an icon made before the loop is running misbehaves against full-screen applications.
  Windows does not care, so one arm serves both. The shell drops the icon in `LoopDestroyed`, and that
  takes the icon out of the bar rather than leaving a ghost.
- **Windows reads the picture out of the executable's own resources**, the same route the title bar
  takes: no decoder, no second copy. **macOS decodes**, because the remote ships there as a bare
  executable with no bundle to read it back out of. That is the crate's only `image` dependency, under
  a target `cfg` table.
- **The decode fixes the height and lets the width follow.** A menu bar gives every item the same
  height and as much width as it asks for. So height is the one dimension a mark has to match.
  Resizing to a square would scale the two axes by different amounts, and it would draw anything that
  is not square stretched. Three of the four marks are square tiles and cannot tell the difference;
  the machine's streaming mark is a pair of letters and does.
- **`Spec::icon_is_template` says the picture is a silhouette macOS may color**, and every glyph
  beside it in that bar is one. Only the machine's streaming mark passes `true`. The three tiles
  cannot, because a tile's silhouette is the tile. It is a property of the picture rather than of the
  platform, so it is a field rather than a `cfg`. Windows has no such idea and ignores it.
- **`tray-icon` is declared per target, naming Windows and macOS rather than excluding Linux.** The
  crate is a workspace member, so a workspace build compiles it everywhere. The negative spelling
  would ask for `libayatana-appindicator` on the BSDs. Nothing here builds for that platform, and
  nobody would notice until it failed. `build` still exists everywhere and answers with an error, so
  no caller needs a `#[cfg]`.
- **The menu's shape is a pure function** of the `Spec`: whether there is a window, and which pages
  the run serves. The id mapping is a pure function of a `&str`. That puts the rules worth pinning
  inside tests that run on a machine with no icon bar, which is every machine CI uses. The tests pin
  these rules:
  - `Show` is absent rather than grayed when there is no window.
  - An entry is absent for a page the run does not serve.
  - Every id round-trips to its command, and some menu offers every command.
  - A left click asks for something that its own menu contains.
- **A left click is decided beside the menu rather than inside the builder.** The two can disagree,
  and the disagreement is silent. A click that sends a command the menu never offers reaches a shell
  with no reason to handle it, and nothing happens at all. That is not hypothetical. The click fell
  back to `Open in browser` whenever there was no window, and that is exactly the entry the machine's
  menu does not have.

### The macOS application menu

**`km-tray` owns it too, and the crate's name is the only thing that makes that surprising.** It is
one function beside the icon's, `install_app_menu`, for one reason. `muda`'s
`MenuEvent::set_event_handler` is a process-wide slot. Its *first* writer wins, and it discards the
later ones rather than merging them. Two menus therefore have to be one registration.

So both route through a single forwarder held in a `OnceLock`, and the app menu's Quit carries the
tray's own id. The installed handler captures nothing and looks that forwarder up, and that stops the two "once"s
from disagreeing about which call won.

**The two calls are separate rather than one folded into the other.** They are two platform objects,
wanted separately and failing separately. A `Spec` describes an icon, and a menu bar uses none of it.
A run can be asked to go without the icon. One call answering for both could report only one failure,
where a caller needs to know which of the two it lost.

**A run with no window gets the menu too.** The loop is entered whether or not there will be a
window. A `--browser` or `--lan` run is a server listening with nothing on screen. It is still an
application in the Dock, and ⌘Q stops it through the same shutdown. Measured on macOS:
`km-remote --browser` shows the bar and quits on ⌘Q with `stopping` in the log. That is the graceful
path rather than a `terminate:`.

**Quit is a `MenuItem` and never `PredefinedMenuItem::quit`.** The predefined one is `terminate:`,
which reaches `applicationWillTerminate`. So `tao` does still emit `LoopDestroyed`, and the teardown
does run. But it never passes through the handler, and the handler is where each shell asks its
*server* to stop. The visible symptom would be the remote sitting out its shutdown grace, waiting for
an outcome nothing started.

**It is installed on the way into the loop, where the icon is installed from inside it.** The icon's
timing is `tray-icon`'s rule about status items and full-screen applications. A menu bar has no such
rule: it needs only the `NSApplication` that `EventLoopBuilder::build` has already arranged. Nothing
removes it on the way out. `tao::run` calls `process::exit`, so the menu bar goes with the process.

### A console twin says it wants its console

**`km-console` takes a `Console::{NotWanted, Wanted}` and the executable says which it is.** A
decision on the console's **process count** alone is wrong for half the pairs. One process means this
process is alone in a console, and a double-click arranges exactly that. That is right for an
executable handed a console it never asked for. **It is exactly backwards for the one whose reason to
exist is to be read.** That one frees the console and closes the window a moment after it opened.

**Nothing observable separates the two.** Both halves of a pair can be double-clicked, and both get
the same console. The subsystem is a flag in a PE header that the process would have to read about
itself.

The rule lives *outside* the `cfg(windows)` module so a test can pin it on every platform. Two of the
three CI platforms could not otherwise check it.

A double-clicked twin has somewhere to talk, so it prints its address instead of opening a browser
tab.

## Where a window opens

`km-webshell` holds the one thing the three shells' `desktop.rs` files agreed about byte for byte. That
is the arithmetic that clamps a wanted window to the screen and centres it on the right monitor. It
has no `run`, no `WebView` and no `wry`. The crate that decides where a window opens does not drag a
browser engine in with it.

**`tao` is declared per target there for the same reason `tray-icon` is in `km-tray`, and the cost of
learning that twice was 113 commits.** A plain `[dependencies]` entry made `cargo clippy --workspace`
and `cargo test --workspace` fail on Linux inside `pango-sys`. `tao`'s Linux backend is gtk3, and
`tools/platform/linux/apt-deps.sh` installs none of it. Both commands failed before either reached a
line of this repository's own code. **A feature would not have helped.** The three shells already
gate the crate behind their own `desktop`, and `--workspace` builds a member whatever its dependents
asked for.

**What is left on Linux is the part worth having there.** `fit` and `centre` speak no `tao`, so they
and their tests compile and run on every platform. That is why the crate stays a workspace member
rather than leaving it. Only `opening_geometry` is `cfg`-ed out. Unlike `km-tray`'s `build`, it gets
no stub arm on the other platforms. It takes a `tao` type, so no signature can stub it without the
dependency.

**Only a Linux `--workspace` build sees it**: CI's `linux` job and `tools/platform/linux/check.sh`.
`cargo km-test` on Windows compiles `tao` against WebView2's own backend and is happy.

## The webview's own profile

**Both windowed programs name their webview's user data folder.** A bare builder leaves WebView2 to
choose, and its choice is `<exe name>.WebView2` **beside the executable**. That is worst where both
programs sit together in a staged folder, and worse again where that folder is not writable.

**Only WebView2 reads the field, and the code is shaped to say so.** `wry` consumes the data-directory
option in exactly two places: the GTK backend, which this project never builds, and the WebView2 one.
The macOS implementation is a unit struct that discards the path, because WKWebView keeps its state
in the application's own container. So the helper is a `cfg(windows)` / `cfg(not(windows))` pair
returning `None` off Windows. **A path returned there would create a directory nothing would ever
open.**

**The directory is created here rather than left to WebView2.** So a failure becomes this function's
`None`, which means a wry default and a window. The alternative is an environment that will not build,
and a run that falls back to a browser it did not need. `None` never stops the window opening.

**The context is a local and is dropped when the function returns.** That looks wrong against wry's
warning that dropping one loses custom-protocol actions on macOS, and it is not. The builder unifies
the context borrow with the window's and returns a webview carrying no lifetime, so both end there.
Neither program registers a custom protocol.

**Two crates, two copies, deliberately.** These files are acknowledged verbatim twins and share only
the tray crate; fifteen lines apiece is the cheaper side of that trade. **The thing that must not
drift is the directory qualifier**, and each file uses the one its own crate already uses elsewhere.

The tests decline to assert `Some`, because a platform that will not name a home directory is a
legitimate `None`. **What they pin is what would regress silently.** A relative path, or one under the
executable, would both still work. Both would put the profile back where this exists to move it from.

**A named profile keeps an HTTP cache, and that is what makes a landing redirect a promise.** The
cache outlives the run that filled it. So the shell follows what the server answered at a URL on the
next launch, rather than asking again. A *permanent* redirect is the one answer a browser is entitled
to keep for good.

A shell therefore opens the page it exists to show, rather than an origin that
redirects to it. A landing redirect that the server serves at all is temporary. Those two rules live
in `Bound::front_door` and in the `/` route in `km-admin`'s `server.rs`. They prevent a window that
opens on a path the build does not mount.
