//! An icon in the OS icon bar — the Windows notification area, the macOS menu bar — for a program
//! that starts a web server and then gets out of the way.
//!
//! **What it is for is the runs that have nothing else to show for themselves.** `km-package-builder`
//! and `km-remote` both put their page in a window on Windows and macOS, and a window is its own
//! answer to *is this running* and *how do I stop it*. But `--browser`, `--lan` and the fallback
//! taken when WebView2 is missing all leave a server listening with no window — and on a
//! GUI-subsystem executable, no console either. Such a process is invisible: nothing says it is
//! there and nothing but Task Manager will stop it. An icon in the bar is the platform's own answer
//! to both questions.
//!
//! **Closing the window still quits**, and this does not change that. There is deliberately no
//! minimize-to-tray here: a program the user believes they closed is not left running behind an icon
//! they may not have noticed. See the `A running server has an icon in the bar` decision in
//! docs/decisions/interface.md.
//!
//! # Two menus, because one of the four serves more than one page
//!
//! **A tool has one page and the karaoke machine has three**, so [`Pages`] says which of the two
//! menus a run is asking for. A tool's entry is named for the act — `Open in browser` — because the
//! address directly above it already names the page it opens. The machine's three are named for the
//! pages themselves, because there it is a real question which one you want: the singer's remote,
//! the screen the stream is on, or the owner's setup.
//!
//! **The crate knows which entries a run offers and the shell knows where they point.** Nothing
//! here opens anything or holds a second address — [`Spec::url`] is in the menu as a label, not as a
//! destination — so this stays a crate about an icon rather than one about the karaoke machine.
//!
//! # The shape of it
//!
//! The caller owns the event loop; this crate owns the icon. [`build`] takes a plain closure and
//! calls it when somebody picks something, so **nothing here depends on `tao` or `wry`** — the two
//! shells wire the closure to their own `EventLoopProxy`. That is a deliberate seam rather than
//! tidiness: `crates/remote/km-remote/src/desktop.rs` already records that its `tao` is kept for
//! uniformity rather than need, and a tray that named a window library would be a second thing to
//! change if that ever moved.
//!
//! Four things about the platforms are not obvious:
//!
//! - **The icon must be created after the event loop is running**, not before it. `tray-icon` says
//!   so for macOS specifically — the earliest safe moment is the equivalent of `StartCause::Init`,
//!   and creating one before that misbehaves against full-screen applications. Both shells therefore
//!   call [`build`] from inside their loop rather than on the way into it.
//! - **On Windows the picture comes out of the executable's own resources**, exactly as
//!   `desktop.rs`'s `window_icon` does: `LoadImageW` on the ordinal `winresource` wrote. No decoder,
//!   and no second copy of a picture to keep in step with `icon/`.
//! - **On macOS it is decoded**, because there is nothing to read it back out of: a Mach-O has no
//!   resource section a `LoadImageW` equivalent could reach, so the picture has to come from a PNG
//!   whichever way the program was shipped. That is the only reason this crate has an `image`
//!   dependency, and why it is under a `cfg(target_os = "macos")` table. **The bundle is not what
//!   decides it**: `KM Remote.app` is staged there for the macOS installer's `remote` component,
//!   and an `.icns` in `Contents/Resources` is what the Finder and the Dock draw, while the menu
//!   bar wants 44 physical pixels out of a PNG.
//! - **And a macOS menu bar draws template images**, which is what the glyphs beside this one are:
//!   the system reads a picture's alpha and paints the shape itself, so one file suits a light bar,
//!   a dark one and an open menu. A full-bleed tile cannot be one — its silhouette is the tile — so
//!   [`Spec::icon_is_template`] is how the one caller with a mark drawn as a silhouette says so, and
//!   the three with a tile take a colored icon.
//!
//! Linux has no tray here, on the same terms as it has no window: `tray-icon`'s only backend there
//! links `libayatana-appindicator` at load time, which is the dependency the
//! `The package builder's window` decision refuses for `wry`. [`build`] is still *present* on every
//! platform and answers with an error, so a caller needs no `#[cfg]` of its own.
//!
//! # The other menu, and why it is here too
//!
//! **macOS also wants an *application* menu, and a shell without one has no ⌘Q.** On that platform
//! ⌘Q is not a key a window is sent: it is a key equivalent on an item of `NSApp.mainMenu`, and
//! `tao` sets no menu at all — which takes every standard shortcut that lives on one with it, ⌘C and
//! ⌘V in the page's own text fields included. [`install_app_menu`] is that menu, and it is
//! macOS-only: Windows has no application menu bar and Linux has no window. The karaoke machine
//! needs none of it, SDL building its own.
//!
//! **It is in this crate rather than in the three shells because of one line inside `muda`.**
//! `MenuEvent::set_event_handler` writes a `OnceCell` and throws the result away, so the *second*
//! call in a process is silently discarded. A shell installing a handler of its own beside this
//! crate's would compile, run, and leave ⌘Q doing nothing — the same silence the menu exists to
//! end, from a different cause. So there is one handler, installed by [`route_menu_events`] on
//! behalf of whichever of [`build`] and [`install_app_menu`] runs first, and the app menu's Quit
//! carries [`id_of`]`(Command::Quit)` so that handler already knows it.
//!
//! **The consequence worth stating: ⌘Q is not a fourth way out.** It is the same [`Command::Quit`]
//! the tray's menu sends, reaching the same shutdown closing the window asks for.
//!
//! **The two are installed by separate calls rather than one doing both**, because they are two
//! platform objects that fail separately and are wanted separately. A [`Spec`] describes an icon —
//! an address to label it with, a picture, whether there is a window to raise — and a menu bar uses
//! none of that; the icon can be declined by a run that still has to be quittable; and one call
//! answering for both could only report one failure, leaving a caller unable to say which of the
//! two it lost. They share a handler, not a call.

use anyhow::Result;

// **Three private helpers below carry a `cfg_attr` allowing `dead_code`, and the reason is the same
// for all three**, so it is given here rather than three times. `id_of`, `command_for` and `tooltip`
// are reached only from the real [`build`] and from the tests — so on a platform with no icon bar
// they are dead in a non-test build, and the workspace lints with `-D warnings`, which would fail
// `tools/platform/linux/check.sh` on a crate working exactly as designed. They are deliberately *not* moved
// inside a `#[cfg]` block with `build`: they are the part worth testing, and a test that only runs
// on Windows is a test two of the three CI platforms cannot run.

/// What somebody asked the tray for.
///
/// Each maps onto something the shell can already do — there is deliberately nothing here that only
/// exists in the menu. `Quit` is the *same* shutdown closing the window asks for, not a second one
/// beside it.
///
/// **Flat, and the four that open a page are four variants rather than one carrying which page.**
/// What makes this design safe is that a shell forgetting an entry does not compile, and a single
/// `Open(page)` variant takes that away: `Open(_) => open_browser(&url)` reads like careful code and
/// would swallow every page added after it. Separate variants cannot be collapsed that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Bring the window to the front. Only ever offered when there is one.
    Show,
    /// Open the served page in the platform's browser. The three tools' one entry.
    OpenInBrowser,
    /// Open the singer's remote. The machine's root, and the machine's alone.
    OpenTheRemote,
    /// Open the page the stream is watched on.
    OpenTheWatchPage,
    /// Open the owner's page, where the machine is set up.
    OpenTheSetupPage,
    /// Stop the server and end the process.
    Quit,
}

/// Which pages a run serves, and therefore which entries its menu offers.
///
/// **A closed enum rather than a list the caller composes.** A sequence would admit an empty menu, a
/// second `Watch`, and `Setup` above `Remote` — each of those a decision about what the product
/// offers and in what order, moved out to four call sites where no test can see it. [`items_for`] is
/// the single source of the menu's shape and stays so; what a caller chooses here is which program
/// it is, which is the only part a caller knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pages {
    /// One page, which the address at the top of the menu already names.
    JustThisOne,
    /// The karaoke machine's three.
    RemoteWatchAndSetup,
}

/// One line of the menu, decided before any of it is built.
///
/// Split out from the building so the one rule worth pinning — that `Show` is absent, rather than
/// grayed out, when there is no window — can be tested on a machine with no icon bar at all, which
/// includes every machine CI runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// The address, drawn but not clickable: a label saying where the thing is.
    Address,
    /// A rule between groups.
    Separator,
    /// Something to pick.
    Command(Command),
}

/// The icon resource a program with one icon has.
///
/// `winresource`'s `DEFAULT_APPLICATION_ICON_ID`, which is what `set_icon` writes and what the
/// Windows shell draws for the executable itself. A program that attaches a second mark gives it an
/// ordinal above this one, so the shell still finds the first.
pub const DEFAULT_ICON_ORDINAL: u16 = 1;

/// What to put in the bar.
#[derive(Debug, Clone)]
pub struct Spec {
    /// The product's name, as it appears in the tooltip.
    pub title: &'static str,
    /// Where the server is listening, e.g. `http://127.0.0.1:8178/`.
    pub url: String,
    /// The product's own mark, as a PNG. Read on macOS and ignored on Windows, which has the same
    /// picture compiled into the executable already.
    pub icon_png: &'static [u8],
    /// Whether that PNG is a silhouette the platform may color itself.
    ///
    /// **macOS reads it and Windows has no such idea.** A template image is drawn from its alpha
    /// alone, so the system paints it to suit a light bar, a dark bar and an open menu; the
    /// notification area draws whatever picture it is given and inverts nothing.
    ///
    /// **A property of the picture, not of the platform**, which is why it is a field rather than a
    /// `cfg`. A full-bleed tile's silhouette is the tile, so a caller that hands one over and says
    /// `true` gets a solid square; only a mark drawn as a silhouette may say so.
    pub icon_is_template: bool,
    /// Which of the executable's icon resources holds that same mark. Read on Windows and ignored
    /// on macOS, which has no resources to read.
    ///
    /// [`DEFAULT_ICON_ORDINAL`] is what a program with one icon passes, and three of the four here
    /// do. The machine passes a second ordinal for a streaming run, because the two ways of
    /// starting it wear different marks and only one of them can be the ordinal the shell draws for
    /// the executable itself.
    pub icon_ordinal: u16,
    /// Whether this run has a window, and therefore whether `Show` is worth offering.
    pub has_a_window: bool,
    /// Which pages this run serves, and therefore which entries open them.
    pub pages: Pages,
}

/// The icon, for as long as it should be in the bar.
///
/// Dropping it takes the icon away, so a caller holds it for the life of the event loop. It is
/// deliberately not `Send`: on Windows and macOS alike the icon belongs to the thread whose event
/// loop created it.
///
/// **The address line and the title are kept because [`Tray::set_url`] needs both**, and it is the
/// one part of the menu a run can move: an address is a fact about the network, and a machine's
/// network is not a fact for the evening.
pub struct Tray {
    #[cfg(any(windows, target_os = "macos"))]
    _icon: tray_icon::TrayIcon,
    /// The disabled item at the top of the menu, held so its text can be replaced.
    #[cfg(any(windows, target_os = "macos"))]
    address: tray_icon::menu::MenuItem,
    /// The product's name, which the tooltip needs beside a new address.
    #[cfg(any(windows, target_os = "macos"))]
    title: &'static str,
}

impl Tray {
    /// Draws a different address, in the menu and in the tooltip together.
    ///
    /// **Both or neither.** They are one fact shown twice, and a caller that could move one without
    /// the other would be a caller that can make the icon contradict itself.
    #[cfg(any(windows, target_os = "macos"))]
    pub fn set_url(&self, url: &str) {
        self.address.set_text(url);
        self._icon.set_tooltip(Some(tooltip(self.title, url))).ok();
    }

    /// The same where there is no icon bar. [`build`] refuses on those platforms, so nothing can
    /// hold a `Tray` to call this on; it exists so a shell needs no `cfg` of its own.
    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn set_url(&self, _url: &str) {}
}

/// The menu this spec asks for, top to bottom.
///
/// `Show` is **absent** rather than disabled when there is no window, on the rule the rest of this
/// project follows for the same question: a command that cannot work is better missing than grayed
/// out, because a grayed-out one invites the user to work out what would enable it.
///
/// **[`Pages`] is that same rule on a second axis.** A tool's menu carries none of the machine's
/// three, and the machine's carries no `Open in browser`, for the reason `Show` is missing from a
/// windowless run: there is no such page for the entry to open.
pub fn items_for(spec: &Spec) -> Vec<Item> {
    let mut items = vec![Item::Address, Item::Separator];
    if spec.has_a_window {
        items.push(Item::Command(Command::Show));
    }
    match spec.pages {
        Pages::JustThisOne => items.push(Item::Command(Command::OpenInBrowser)),
        Pages::RemoteWatchAndSetup => {
            // The order is the product decision this function exists to hold: the remote first
            // because it is what the address above names and what most hands reach for, and the
            // owner's page last because it is the one nobody opens twice in an evening.
            items.push(Item::Command(Command::OpenTheRemote));
            items.push(Item::Command(Command::OpenTheWatchPage));
            items.push(Item::Command(Command::OpenTheSetupPage));
        }
    }
    items.push(Item::Separator);
    items.push(Item::Command(Command::Quit));
    items
}

/// What a plain left click on the icon asks for, where the platform lets us say.
///
/// With a window it raises it; without one there is nothing to raise, so the click does the most
/// useful thing that menu offers instead.
///
/// **Beside [`items_for`] rather than inside [`build`], so the one rule that binds the two can be
/// tested on a machine with no icon bar.** The rule is that a click must ask for something the menu
/// offers, and breaking it is silent: the icon is still there, the click still sends a command, and
/// the shell that was never given a reason to handle it does nothing at all.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn left_click(spec: &Spec) -> Command {
    if spec.has_a_window {
        return Command::Show;
    }
    match spec.pages {
        Pages::JustThisOne => Command::OpenInBrowser,
        Pages::RemoteWatchAndSetup => Command::OpenTheRemote,
    }
}

/// One line of the macOS application menu bar, decided before any of it is built.
///
/// Split out from the building on exactly the terms [`Item`] is: the shape is the part worth
/// pinning, and pinning it must not need a menu bar to run against — which is every machine CI uses
/// and two of the three platforms this crate compiles for.
///
/// **All but the last are the platform's own actions rather than this program's**, which is what
/// makes them safe to hand to `muda` as predefined items: each is a first-responder selector AppKit
/// routes to whatever has focus — the webview's text field, for the editing six — and none of them
/// ends the process behind the event loop's back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarItem {
    /// A rule between groups.
    Separator,
    /// `About <product>`, which AppKit fills in from the bundle.
    About,
    /// `Hide <product>`.
    Hide,
    /// `Hide Others`.
    HideOthers,
    /// `Show All`.
    ShowAll,
    /// `Undo`.
    Undo,
    /// `Redo`.
    Redo,
    /// `Cut`.
    Cut,
    /// `Copy`.
    Copy,
    /// `Paste`.
    Paste,
    /// `Select All`.
    SelectAll,
    /// The one line in either menu that is this program's rather than the platform's.
    Command(Command),
}

/// The application submenu — the one under the product's name — top to bottom.
///
/// **It ends in [`Command::Quit`] and not in `muda`'s predefined Quit, and that is the whole point
/// of the function.** The predefined one is `terminate:`, which reaches `applicationWillTerminate`
/// and so does run each shell's `LoopDestroyed` — but it never asks the *server* to stop, because
/// asking is what these shells do on the way *into* the exit and `terminate:` walks past it. The
/// remote would then sit out its whole shutdown grace waiting for an outcome nobody started. A
/// command routed the ordinary way is what makes ⌘Q identical to the three exits already here
/// rather than a fourth that merely looks like them.
pub fn app_menu_items() -> Vec<BarItem> {
    vec![
        BarItem::About,
        BarItem::Separator,
        BarItem::Hide,
        BarItem::HideOthers,
        BarItem::ShowAll,
        BarItem::Separator,
        BarItem::Command(Command::Quit),
    ]
}

/// The Edit submenu, top to bottom.
///
/// **Nothing in the program reads these and that is why they work.** They are here because an empty
/// menu bar takes ⌘C, ⌘V, ⌘X and ⌘A down with ⌘Q — the page served into the webview has text fields,
/// and on macOS those shortcuts arrive through this menu or not at all.
pub fn edit_menu_items() -> Vec<BarItem> {
    vec![
        BarItem::Undo,
        BarItem::Redo,
        BarItem::Separator,
        BarItem::Cut,
        BarItem::Copy,
        BarItem::Paste,
        BarItem::SelectAll,
    ]
}

/// What a command is called in the application menu, which is not what it is called in the tray's.
///
/// **`Quit KaraokeMachine Remote` there, `Quit` in the tray**, and the difference is the
/// surrounding: the tray's menu hangs off this program's own icon and says the address at the top,
/// while the application menu is one of a dozen in a bar and macOS's convention is that this item
/// names the application.
/// AppKit fills that in for its own predefined items and not for ours, a custom item's title being
/// whatever it is given.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn menu_bar_label(command: Command, app_name: &str) -> String {
    match command {
        Command::Quit => format!("Quit {app_name}"),
        // None of these is offered in the application menu today. Spelled rather than left to a
        // wildcard so that adding a command to that menu is a compile error here instead of a line
        // reading the tray's label in a bar that wanted the product's name in it.
        Command::Show
        | Command::OpenInBrowser
        | Command::OpenTheRemote
        | Command::OpenTheWatchPage
        | Command::OpenTheSetupPage => label_of(command).to_owned(),
    }
}

/// The id each command's menu entry carries.
///
/// Fixed strings rather than the generated ids `MenuItem::new` hands out, so that the mapping back
/// is a pure function of a `&str` and can be tested without building a menu.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn id_of(command: Command) -> &'static str {
    match command {
        Command::Show => "km-tray.show",
        Command::OpenInBrowser => "km-tray.browser",
        Command::OpenTheRemote => "km-tray.remote",
        Command::OpenTheWatchPage => "km-tray.watch",
        Command::OpenTheSetupPage => "km-tray.setup",
        Command::Quit => "km-tray.quit",
    }
}

/// Every command there is, for the two places that have to walk the whole set.
///
/// **One list rather than two literals a hundred lines apart**, because what both of them are for is
/// being complete: [`command_for`] maps an id back only if this names the command, and the tests
/// assert over this. A variant left out of a copy would fail neither.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
const EVERY_COMMAND: [Command; 6] = [
    Command::Show,
    Command::OpenInBrowser,
    Command::OpenTheRemote,
    Command::OpenTheWatchPage,
    Command::OpenTheSetupPage,
    Command::Quit,
];

/// Which command a menu id names, if any.
///
/// `None` is an ordinary answer and not a defect: `muda`'s event handler is process-wide, so an id
/// this tray never created can arrive here and the right thing to do with it is nothing.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn command_for(id: &str) -> Option<Command> {
    EVERY_COMMAND
        .into_iter()
        .find(|command| id_of(*command) == id)
}

/// What the tooltip says: the product, and where it is.
///
/// **The two parts rather than the [`Spec`] holding them**, because [`Tray::set_url`] writes a
/// tooltip long after the spec it was built from has gone.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn tooltip(title: &str, url: &str) -> String {
    format!("{title} — {url}")
}

/// Puts the icon in the bar. Call it from inside a running event loop.
///
/// `on` is called on the platform's own event thread whenever somebody picks something, and both
/// shells forward it to their event loop rather than acting there — a menu handler is not a good
/// place to close a window from.
///
/// **A failure here is not fatal and callers must not treat it as one.** An icon that could not be
/// created is a blemish; a program that refuses to run over one is a bug. That is the same rule
/// `desktop.rs`'s `window_icon` already follows.
#[cfg(any(windows, target_os = "macos"))]
pub fn build(spec: Spec, on: impl Fn(Command) + Send + Sync + 'static) -> Result<Tray> {
    // Inside the function rather than at the top of the file, and `Arc` with them: a platform with
    // no icon bar compiles none of this, and an import it cannot use is a warning the workspace
    // treats as an error.
    use std::sync::Arc;

    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::new();
    // Filled in by the loop below, which appends exactly one `Item::Address`. `items_for` puts it
    // first and unconditionally, so this is never still `None` afterwards — and the `expect` says
    // so rather than a `Tray` with no address to move.
    let mut address = None;
    for item in items_for(&spec) {
        match item {
            // Disabled on purpose: it is the address, there to be read rather than clicked. The
            // thing to click for that is the first entry below the rule.
            Item::Address => {
                let item = MenuItem::new(&spec.url, false, None);
                menu.append(&item)?;
                address = Some(item);
            }
            Item::Separator => menu.append(&PredefinedMenuItem::separator())?,
            Item::Command(command) => menu.append(&MenuItem::with_id(
                id_of(command),
                label_of(command),
                true,
                None,
            ))?,
        }
    }

    // Shared by the click handler below and by whichever menu got there first. Both of the setters
    // in play are process-wide `OnceCell`s inside `muda` and `tray-icon`, so each can only take
    // effect once per process — which is why the menu one is `route_menu_events`' business and not
    // this function's.
    let on: Arc<dyn Fn(Command) + Send + Sync> = Arc::new(on);

    route_menu_events(Arc::clone(&on));

    // What a plain left click does, where the platform lets us say. Decided beside `items_for` so
    // that the two cannot come to disagree about what this menu holds.
    let clicked = left_click(&spec);
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            on(clicked);
        }
    }));

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon_as_template(spec.icon_is_template)
        .with_tooltip(tooltip(spec.title, &spec.url));

    if let Some(icon) = icon(spec.icon_png, spec.icon_ordinal) {
        builder = builder.with_icon(icon);
    }

    // **Windows only, and the asymmetry follows each platform's convention.** There a left
    // click on a tray icon opens the thing and a right click opens its menu; on macOS a click on a
    // status item *is* how you open its menu, and taking that away would make the menu unreachable
    // for anybody who does not know to right-click a menu bar.
    #[cfg(windows)]
    {
        builder = builder.with_menu_on_left_click(false);
    }

    Ok(Tray {
        _icon: builder.build()?,
        address: address.expect("items_for always starts with the address"),
        title: spec.title,
    })
}

/// Everywhere else there is no bar to put an icon in. See the module documentation for why Linux is
/// one of those places.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn build(_spec: Spec, _on: impl Fn(Command) + Send + Sync + 'static) -> Result<Tray> {
    anyhow::bail!("this platform has no icon bar")
}

/// Where a picked menu item is sent, whichever menu it was picked from.
///
/// A `OnceLock` rather than a parameter threaded through both builders, because the thing it stands
/// in for — `muda`'s own handler slot — is process-wide too, and two ways of saying "once" that
/// could disagree would be worse than one that cannot.
#[cfg(any(windows, target_os = "macos"))]
static FORWARD: std::sync::OnceLock<std::sync::Arc<dyn Fn(Command) + Send + Sync>> =
    std::sync::OnceLock::new();

/// Points `muda`'s one handler at `on`, if nothing has pointed it anywhere yet.
///
/// **Called by [`build`] and by [`install_app_menu`], and the first of them wins.** Both hand over
/// the same thing — a forwarder onto the one event loop this process has — so *which* one wins does
/// not matter; that exactly one does is what matters. `MenuEvent::set_event_handler` writes a
/// `OnceCell` and *discards* the result, so a second handler is not an error, an override or a
/// second subscriber: it is silence.
///
/// **The handler deliberately captures nothing and looks the forwarder up instead.** That is what
/// stops the two `OnceCell`s being able to disagree: whichever call loses the race here also loses
/// it inside `muda`, and the handler that does get installed reads whichever won.
#[cfg(any(windows, target_os = "macos"))]
fn route_menu_events(on: std::sync::Arc<dyn Fn(Command) + Send + Sync>) {
    use tray_icon::menu::MenuEvent;

    let _ = FORWARD.set(on);
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        // `None` is an ordinary answer: the handler is process-wide, so an id neither of this
        // program's two menus created can arrive, and the right thing to do with it is nothing.
        if let Some(command) = command_for(event.id.as_ref())
            && let Some(forward) = FORWARD.get()
        {
            forward(command);
        }
    }));
}

/// The application menu, for as long as it should be in the bar.
///
/// Dropping it does *not* take the menu away — `init_for_nsapp` hands it to `NSApp`, which keeps a
/// reference of its own — so this is held for the reason the shells hold [`Tray`]: to say in the
/// type system that the menu belongs to the run rather than to the moment it was made.
pub struct AppMenu {
    #[cfg(target_os = "macos")]
    _menu: tray_icon::menu::Menu,
}

/// Puts an application menu in the macOS menu bar.
///
/// **Unlike [`build`], this is called on the way *into* the loop rather than from inside it**, and
/// the difference is worth stating because the two look like they should match. The icon's timing is
/// `tray-icon`'s own requirement about status items and full-screen applications; a menu bar has no
/// such rule — `init_for_nsapp` needs only that an `NSApplication` exists, which `tao` has arranged
/// by the time `EventLoopBuilder::build` returns. Installing it before `run` is also what puts the
/// menu in the bar for the first frame the window is on screen, rather than a beat later.
///
/// `on` is called on the platform's own event thread, exactly as [`build`]'s is, and callers forward
/// rather than act there for the same reason.
///
/// **A failure here is not fatal and callers must not treat it as one**, on the same terms as
/// [`build`]: a program that refuses to serve over a menu it could not build is a bug. What is lost
/// is ⌘Q and the editing shortcuts; the window's close button, the icon in the bar and Ctrl-C are
/// all still there.
#[cfg(target_os = "macos")]
pub fn install_app_menu(
    app_name: &str,
    on: impl Fn(Command) + Send + Sync + 'static,
) -> Result<AppMenu> {
    use std::sync::Arc;

    use tray_icon::menu::{Menu, Submenu};

    route_menu_events(Arc::new(on));

    let bar = Menu::new();

    // **The first submenu is the application menu, whatever it is called.** AppKit takes the name it
    // draws in bold from the bundle and ignores this title — but a `Submenu` has to be given one,
    // and the product's own name is the least surprising thing to find in a debugger.
    let app = Submenu::new(app_name, true);
    bar.append(&app)?;
    for item in app_menu_items() {
        append_bar_item(&app, item, app_name)?;
    }

    let edit = Submenu::new("Edit", true);
    bar.append(&edit)?;
    for item in edit_menu_items() {
        append_bar_item(&edit, item, app_name)?;
    }

    bar.init_for_nsapp();

    Ok(AppMenu { _menu: bar })
}

/// Everywhere else there is no application menu to put anything in.
///
/// **Windows is one of those places as much as Linux is**, which makes this the one function here
/// whose two halves split on macOS rather than on Linux: that platform has an icon bar and no
/// application menu bar, and a per-window menu is not the same thing and not wanted.
#[cfg(not(target_os = "macos"))]
pub fn install_app_menu(
    _app_name: &str,
    _on: impl Fn(Command) + Send + Sync + 'static,
) -> Result<AppMenu> {
    anyhow::bail!("this platform has no application menu")
}

/// Puts one line into one submenu.
#[cfg(target_os = "macos")]
fn append_bar_item(into: &tray_icon::menu::Submenu, item: BarItem, app_name: &str) -> Result<()> {
    use tray_icon::menu::{MenuItem, PredefinedMenuItem};

    match item {
        BarItem::Separator => into.append(&PredefinedMenuItem::separator())?,
        // `None` throughout: each of these takes an optional title override, and the platform's own
        // wording — translated, and with the product's name already in it where AppKit puts one — is
        // better than anything that could be spelled here.
        BarItem::About => into.append(&PredefinedMenuItem::about(None, None))?,
        BarItem::Hide => into.append(&PredefinedMenuItem::hide(None))?,
        BarItem::HideOthers => into.append(&PredefinedMenuItem::hide_others(None))?,
        BarItem::ShowAll => into.append(&PredefinedMenuItem::show_all(None))?,
        BarItem::Undo => into.append(&PredefinedMenuItem::undo(None))?,
        BarItem::Redo => into.append(&PredefinedMenuItem::redo(None))?,
        BarItem::Cut => into.append(&PredefinedMenuItem::cut(None))?,
        BarItem::Copy => into.append(&PredefinedMenuItem::copy(None))?,
        BarItem::Paste => into.append(&PredefinedMenuItem::paste(None))?,
        BarItem::SelectAll => into.append(&PredefinedMenuItem::select_all(None))?,
        BarItem::Command(command) => into.append(&MenuItem::with_id(
            id_of(command),
            menu_bar_label(command, app_name),
            true,
            accelerator_for(command),
        ))?,
    }
    Ok(())
}

/// The shortcut a command answers to in the application menu.
///
/// **`Modifiers::SUPER` is Command on macOS**, which the name does not say: `muda` maps `SUPER` to
/// `NSEventModifierFlagCommand` and normalizes `META` into it. `CMD_OR_CTRL` is that same constant
/// under a name that says what it is for.
#[cfg(target_os = "macos")]
fn accelerator_for(command: Command) -> Option<tray_icon::menu::accelerator::Accelerator> {
    use tray_icon::menu::accelerator::{Accelerator, CMD_OR_CTRL, Code};

    match command {
        Command::Quit => Some(Accelerator::new(Some(CMD_OR_CTRL), Code::KeyQ)),
        // None of these is in this menu. `Show` has no conventional shortcut, and ⌘O is the
        // open-a-document one rather than an open-a-browser one.
        Command::Show
        | Command::OpenInBrowser
        | Command::OpenTheRemote
        | Command::OpenTheWatchPage
        | Command::OpenTheSetupPage => None,
    }
}

/// What each command is called in the tray's menu.
///
/// Compiled everywhere rather than beside `build`, for the reason the three helpers above give:
/// `menu_bar_label` is built on it and is the part worth testing, and a test that only runs on
/// Windows is a test two of the three CI platforms cannot run.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn label_of(command: Command) -> &'static str {
    match command {
        Command::Show => "Show window",
        Command::OpenInBrowser => "Open in browser",
        // **Named for the page and not for the act**, unlike the one above, because the machine
        // offers three and which one you want is a real question. The words are the ones the rest of
        // the product uses for the same three doors.
        Command::OpenTheRemote => "Remote",
        Command::OpenTheWatchPage => "Watch",
        Command::OpenTheSetupPage => "Setup",
        Command::Quit => "Quit",
    }
}

/// The picture, out of this executable's own resources.
///
/// `winresource`'s `DEFAULT_APPLICATION_ICON_ID` is ordinal 1, which is what each shell's `build.rs`
/// writes — the same pair `desktop.rs` relies on for the title bar, and the same reason
/// [`DEFAULT_ICON_ORDINAL`] is named here rather than spelled at the call site. A build made without
/// `rc.exe` has no resource to find, which is why `None` is an ordinary answer.
#[cfg(windows)]
fn icon(_png: &[u8], ordinal: u16) -> Option<tray_icon::Icon> {
    /// What the notification area draws at, and the frame asked for out of the `.ico`.
    ///
    /// Asked for rather than left to default, for the reason `desktop.rs` gives at length: the
    /// default is `LR_DEFAULTSIZE`, which means the 32-pixel metric, and a 32-pixel drawing squashed
    /// into 16 is the difference between a microphone and a smudge. Every `.ico` here carries an
    /// exact 16 frame.
    const TRAY_ICON: u32 = 16;

    match tray_icon::Icon::from_resource(ordinal, Some((TRAY_ICON, TRAY_ICON))) {
        Ok(icon) => Some(icon),
        Err(error) => {
            tracing::debug!(%error, "no icon in this executable; the tray gets the platform's default");
            None
        }
    }
}

/// The picture, decoded from the mark compiled into the shell.
///
/// macOS has nothing to read it back out of — `km-remote` ships there as a bare executable with
/// no bundle — so this is the one platform that pays for a decoder.
///
/// **A failure is a warning here and a debug line on Windows**, and what each platform does with
/// `None` is why. There a tray icon with no picture draws the platform's own, so the icon is in the
/// bar and holds its menu either way. Here a status item with no image and no title is a blank of
/// almost no width, which is what an icon that was never created also looks like.
#[cfg(target_os = "macos")]
fn icon(png: &[u8], _ordinal: u16) -> Option<tray_icon::Icon> {
    match decode(png) {
        Ok((rgba, width, height)) => match tray_icon::Icon::from_rgba(rgba, width, height) {
            Ok(icon) => Some(icon),
            Err(error) => {
                tracing::warn!(%error, "the mark would not become an icon, so the icon in the bar is blank");
                None
            }
        },
        Err(error) => {
            tracing::warn!(%error, "could not decode the mark, so the icon in the bar is blank");
            None
        }
    }
}

/// The mark as RGBA, as tall as the menu bar and as wide as its own shape asks.
///
/// Resized rather than taken at its stored size, because the marks are generated at power-of-two
/// sizes and the menu bar is 22 points — so on a Retina display the image wanted is 44 pixels, which
/// is not one of them. Downscaling 256 beats upscaling 32.
///
/// **The height is what is fixed, and the width follows from it.** A menu bar gives every item the
/// same height and as much width as it asks for, so height is the one dimension a mark has to match;
/// forcing both would scale the two axes by different amounts, which draws the mark stretched in one
/// of them. That costs nothing while every mark is square — the four tiles are — and it is exactly
/// what a mark that is not square needs, such as a pair of letters with the tile taken off.
///
/// Returned as `(rgba, width, height)`, because [`tray_icon::Icon::from_rgba`] needs both and a
/// caller that could pass one number twice is the caller this exists to stop.
#[cfg(target_os = "macos")]
fn decode(png: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    /// The macOS menu bar at 2×: 22 points.
    const MENU_BAR_ICON: u32 = 44;

    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (width, height) = decoded.dimensions();
    // A mark with no height is not a mark; answering rather than dividing by it keeps this total.
    if height == 0 || width == 0 {
        anyhow::bail!("the mark has no pixels in it");
    }
    // Rounded, and at least one: a mark far wider than it is tall still has to come out with a
    // column in it.
    let wanted = ((width as f32 * MENU_BAR_ICON as f32 / height as f32).round() as u32).max(1);
    let resized = image::imageops::resize(
        &decoded,
        wanted,
        MENU_BAR_ICON,
        image::imageops::FilterType::Lanczos3,
    );
    Ok((resized.into_raw(), wanted, MENU_BAR_ICON))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spec with nothing in it but the two answers the menu's shape is decided by.
    ///
    /// **The literal is written out rather than built from a `Default`**, and that is the point: a
    /// field added to [`Spec`] must be a compile error in every test that makes one, exactly as it
    /// is at the four call sites in earnest. A `..Default::default()` here would let a new axis of
    /// the menu's shape arrive with no test looking at it.
    fn spec(has_a_window: bool, pages: Pages) -> Spec {
        Spec {
            title: "KaraokeMachine",
            url: "http://127.0.0.1:8177".to_owned(),
            icon_png: &[],
            icon_is_template: false,
            icon_ordinal: DEFAULT_ICON_ORDINAL,
            has_a_window,
            pages,
        }
    }

    /// A run with no window offers no `Show`, and one with a window offers exactly one.
    ///
    /// The rule this pins is the one that is easy to get wrong in the other direction: a `Show`
    /// left in the menu of a `--browser` run would raise nothing at all, silently.
    #[test]
    fn show_is_offered_only_when_there_is_something_to_show() {
        let with = items_for(&spec(true, Pages::JustThisOne));
        assert_eq!(
            with.iter()
                .filter(|item| **item == Item::Command(Command::Show))
                .count(),
            1
        );

        let without = items_for(&spec(false, Pages::JustThisOne));
        assert!(!without.contains(&Item::Command(Command::Show)));

        // Everything else is the same either way: the address, and the two commands that always
        // work.
        for items in [&with, &without] {
            assert_eq!(items.first(), Some(&Item::Address));
            assert!(items.contains(&Item::Command(Command::OpenInBrowser)));
            assert_eq!(items.last(), Some(&Item::Command(Command::Quit)));
        }
    }

    /// The two menus, whole and in order.
    ///
    /// **Full-vector equality rather than a handful of `contains`**, because the order is as much
    /// the product decision as the contents are: which entry a hand reaches first is what the
    /// ordering in `items_for` is choosing, and nothing else would notice it being rearranged.
    #[test]
    fn a_tool_names_the_act_and_the_machine_names_its_three_pages() {
        assert_eq!(
            items_for(&spec(false, Pages::JustThisOne)),
            vec![
                Item::Address,
                Item::Separator,
                Item::Command(Command::OpenInBrowser),
                Item::Separator,
                Item::Command(Command::Quit),
            ]
        );

        assert_eq!(
            items_for(&spec(false, Pages::RemoteWatchAndSetup)),
            vec![
                Item::Address,
                Item::Separator,
                Item::Command(Command::OpenTheRemote),
                Item::Command(Command::OpenTheWatchPage),
                Item::Command(Command::OpenTheSetupPage),
                Item::Separator,
                Item::Command(Command::Quit),
            ]
        );
    }

    /// No menu offers a page the run does not serve.
    ///
    /// The `Show` rule on its second axis, and it fails in the direction that is silent: an entry
    /// for a page that is not mounted opens a 404, and a person who pressed it learns nothing about
    /// why.
    #[test]
    fn no_menu_offers_a_page_the_run_does_not_have() {
        let tool = items_for(&spec(true, Pages::JustThisOne));
        for absent in [
            Command::OpenTheRemote,
            Command::OpenTheWatchPage,
            Command::OpenTheSetupPage,
        ] {
            assert!(
                !tool.contains(&Item::Command(absent)),
                "a tool serves one page, so it must not offer {absent:?}"
            );
        }

        let machine = items_for(&spec(false, Pages::RemoteWatchAndSetup));
        assert!(
            !machine.contains(&Item::Command(Command::OpenInBrowser)),
            "the machine names its three pages, so the generic entry is not in its menu"
        );
    }

    /// Each entry that opens a page says which page, and no two say the same thing.
    ///
    /// Stops the next entry being added under a label already in the menu — four lines reading
    /// `Open in browser` would be a menu nobody can use and a test suite that noticed nothing.
    #[test]
    fn every_entry_that_opens_a_page_is_labelled_differently() {
        let labels = [
            Command::OpenInBrowser,
            Command::OpenTheRemote,
            Command::OpenTheWatchPage,
            Command::OpenTheSetupPage,
        ]
        .map(label_of);
        assert!(labels.iter().all(|label| !label.is_empty()));

        let mut distinct = labels.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            labels.len(),
            "two entries carry the same label: {labels:?}"
        );
    }

    /// A left click asks for something the menu it belongs to actually offers.
    ///
    /// **The failure this watches for is silent and was live for one shape of menu**: the click used
    /// to fall back to `Open in browser` whenever there was no window, which the machine's menu does
    /// not contain — so clicking its icon would have sent a command no shell had a reason to handle,
    /// and nothing would have happened at all. Every combination, so a third menu shape cannot
    /// reintroduce it.
    #[test]
    fn a_left_click_asks_for_something_the_menu_offers() {
        for has_a_window in [true, false] {
            for pages in [Pages::JustThisOne, Pages::RemoteWatchAndSetup] {
                let spec = spec(has_a_window, pages);
                let clicked = left_click(&spec);
                assert!(
                    items_for(&spec).contains(&Item::Command(clicked)),
                    "a click sends {clicked:?}, which is not in the menu for \
                     ({has_a_window}, {pages:?})"
                );
            }
        }
    }

    /// Every command is reachable from some menu.
    ///
    /// **This is the one that catches a command wired up everywhere and offered nowhere.** The
    /// compiler forces each shell to handle a new variant and forces `id_of` and `label_of` to name
    /// it; nothing forces [`items_for`] to put it in a menu. That build compiles, every shell is
    /// correct, every label is right, and the entry simply is not there.
    #[test]
    fn every_command_is_offered_by_some_menu() {
        let offered: Vec<Command> = [true, false]
            .into_iter()
            .flat_map(|has_a_window| {
                [Pages::JustThisOne, Pages::RemoteWatchAndSetup]
                    .into_iter()
                    .flat_map(move |pages| items_for(&spec(has_a_window, pages)))
            })
            .filter_map(|item| match item {
                Item::Command(command) => Some(command),
                Item::Address | Item::Separator => None,
            })
            .collect();

        for command in EVERY_COMMAND {
            assert!(
                offered.contains(&command),
                "{command:?} is named everywhere and offered in no menu"
            );
        }
    }

    /// Every command's id maps back to it, and an id from anywhere else maps to nothing.
    ///
    /// The round trip is the whole of it: the two halves are written a hundred lines apart and
    /// nothing but this says they agree. The `None` case matters because the handler is
    /// process-wide — an id this tray never created is a real thing to receive, not a bug.
    #[test]
    fn every_id_maps_back_to_the_command_that_carries_it() {
        for command in EVERY_COMMAND {
            assert_eq!(command_for(id_of(command)), Some(command));
        }
        assert_eq!(command_for("1234"), None);
        assert_eq!(command_for(""), None);
    }

    /// The application menu's Quit is routed, not a second Quit standing beside the routed one.
    ///
    /// **This is the whole of why the menu lives in this crate**, so it is what the test pins:
    /// ⌘Q ends up in the same `Command::Quit` the tray's own menu sends, through the same id and
    /// therefore the same process-wide handler. The failure it is watching for is silent — a Quit
    /// built from `muda`'s predefined item, or from an id of its own, would look right in the bar
    /// and reach nothing.
    #[test]
    fn the_application_menus_quit_is_the_command_the_tray_already_sends() {
        let quit = app_menu_items()
            .into_iter()
            .filter_map(|item| match item {
                BarItem::Command(command) => Some(command),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(quit, vec![Command::Quit], "the app menu carries only Quit");
        assert_eq!(command_for(id_of(Command::Quit)), Some(Command::Quit));
    }

    /// The Edit menu is entirely the platform's, and asks nothing of this program.
    ///
    /// Those items work by being first-responder selectors AppKit routes to whatever has focus. One
    /// that arrived as a [`BarItem::Command`] instead would be asking the event loop to do something
    /// about a text field it cannot see.
    #[test]
    fn nothing_in_the_edit_menu_is_routed_back_to_the_program() {
        let items = edit_menu_items();
        assert!(!items.is_empty());
        assert!(
            !items.iter().any(|item| matches!(item, BarItem::Command(_))),
            "the Edit menu is the platform's own actions and nothing else: {items:?}"
        );
    }

    /// The application menu names the product where the tray's menu does not.
    ///
    /// The fixture is the whole name rather than the abbreviated one, because that is what
    /// `km-remote`'s `TITLE` hands this crate: a menu bar is read one item at a time and gets the
    /// sentence, where an icon gets the nine characters. See `What the product is called`.
    #[test]
    fn quit_names_the_product_in_the_menu_bar_and_not_in_the_tray() {
        assert_eq!(
            menu_bar_label(Command::Quit, "KaraokeMachine Remote"),
            "Quit KaraokeMachine Remote"
        );
        assert_eq!(label_of(Command::Quit), "Quit");
    }

    /// The ids are distinct, which the round trip above would not notice on its own.
    #[test]
    fn no_two_commands_share_an_id() {
        let ids = EVERY_COMMAND.map(id_of);
        let mut distinct = ids.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            ids.len(),
            "two commands share an id: {ids:?}"
        );
    }

    /// The tooltip names the product and where it is, which is the whole job.
    ///
    /// The whole name, for the reason above: a tooltip is one string on its own, hovered
    /// deliberately, and is not competing for width with three siblings the way an icon's label is.
    #[test]
    fn the_tooltip_says_what_this_is_and_where() {
        assert_eq!(
            tooltip("KaraokeMachine Package Builder", "http://127.0.0.1:8178/"),
            "KaraokeMachine Package Builder — http://127.0.0.1:8178/"
        );
        // A machine's, after its network arrived: the same shape, and the product's name is what
        // stays put while the address moves.
        assert_eq!(
            tooltip("KaraokeMachine", "http://192.168.1.42:8177"),
            "KaraokeMachine — http://192.168.1.42:8177"
        );
    }

    /// The address is first and there is exactly one of it, in both menus. `build` reaches for it by
    /// that, so a fourth `Item` put above it would be a panic on a machine with an icon bar and
    /// nothing at all on CI.
    #[test]
    fn every_menu_opens_with_one_address() {
        for pages in [Pages::JustThisOne, Pages::RemoteWatchAndSetup] {
            for has_a_window in [false, true] {
                let items = items_for(&Spec {
                    title: "KaraokeMachine",
                    url: "http://192.168.1.42:8177".to_owned(),
                    icon_png: &[],
                    icon_is_template: false,
                    icon_ordinal: DEFAULT_ICON_ORDINAL,
                    has_a_window,
                    pages,
                });
                assert_eq!(items.first(), Some(&Item::Address), "{pages:?}");
                assert_eq!(
                    items.iter().filter(|item| **item == Item::Address).count(),
                    1,
                    "{pages:?}"
                );
            }
        }
    }

    /// A mark decodes to the height the menu bar draws at, whatever shape it is.
    ///
    /// macOS only, because it is the only platform that decodes anything — Windows reads the icon
    /// out of its own resources and never touches this.
    ///
    /// **Both marks, because the rule is only visible on one of them.** A tile is square, so a
    /// decode that forced a square would pass on it and still draw the machine's letters squashed;
    /// the two together are what says the height is fixed and the width is not.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mark_decodes_to_the_height_the_menu_bar_draws_at() {
        /// What each picture is, so the arithmetic below is checked against the files rather than
        /// against itself.
        const TILE: (u32, u32) = (256, 256);
        const LETTERS: (u32, u32) = (218, 128);

        for (png, (source_width, source_height)) in [
            (
                include_bytes!("../../../../icon/km-package-builder-256.png").as_slice(),
                TILE,
            ),
            (
                include_bytes!("../../../../icon/karaokemachine-bar.png").as_slice(),
                LETTERS,
            ),
        ] {
            let (rgba, width, height) = decode(png).expect("the generated mark decodes");
            assert_eq!(rgba.len(), (width * height * 4) as usize);
            // The one dimension a menu bar fixes. Every mark comes out at it.
            assert_eq!(height, 44);
            // ...and the other follows the picture rather than the first, to within what rounding to
            // a whole pixel costs.
            let wanted = source_width as f32 * 44.0 / source_height as f32;
            assert!(
                (width as f32 - wanted).abs() <= 0.5,
                "{width} is not {wanted} wide"
            );
        }
    }

    /// A picture with no pixels in it is an error rather than a division by zero.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mark_with_no_pixels_is_refused() {
        assert!(decode(&[]).is_err());
    }
}
