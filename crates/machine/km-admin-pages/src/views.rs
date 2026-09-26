//! One struct per rendered page, and the chrome they share.
//!
//! The struct-per-template pattern `km-remote-pages` established: askama's derive implements
//! `Display`, so a page holds its parts as fields and the template writes `{{ part|safe }}`.

use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use km_locale::Locale;

/// **What makes `{{ "tab-songs"|t }}` compile.** askama resolves a custom filter against a module
/// called `filters` in the scope the template was derived in, which is this one.
use km_locale::filters;

/// Renders a template, or says plainly that it could not.
///
/// A template error is a bug in this crate rather than anything a person did, so it is a 500 with
/// the reason in it — the alternative, an empty 200, is the one shape that makes a broken page look
/// like an empty machine.
///
/// The locale goes in here and reaches every nested fragment on its own; see
/// [`km_locale::filters`].
pub fn page<T: Template>(template: &T, locale: Locale) -> Response {
    match template.render_with_values(&filters::values(crate::words::messages(locale))) {
        Ok(body) => Html(body).into_response(),
        Err(error) => {
            tracing::error!(target: crate::LOG_TARGET, %error, "a page would not render");
            // Plain text and not HTML: the reason is a Rust error message, it may hold anything,
            // and a page that is already failing to render is the worst place to be interpolating
            // into markup. There is nothing here for a browser to lay out anyway.
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("This page would not render: {error}"),
            )
                .into_response()
        }
    }
}

/// A page whose body a host wrote, inside this crate's chrome.
///
/// # Why a host needs this at all
///
/// **A host with pages of its own cannot `{% extends %}` this crate's layout.** askama resolves a
/// template path against the *including crate's* `templates/` directory, so `km-admin`'s Home page
/// and its picture-search page have no way to reach `layout.html` — and the tab strip, the heading
/// and the factory-password banner are exactly what they must not draw a second copy of, that copy
/// being the drift this whole seam exists to delete.
///
/// So the chrome is offered as a *wrapper*: the host renders its own body and hands the markup over.
/// The content is `|safe` because it is markup, which is the one thing to be careful about — see
/// [`Shell::content`].
#[derive(Template)]
#[template(path = "shell.html")]
pub struct Shell {
    /// The shared chrome: the strip, the heading, the nag.
    pub chrome: Chrome,
    /// What just happened, if anything.
    pub notice: Option<Notice>,
    /// The host's own markup, rendered into `<main>` **unescaped**.
    ///
    /// # What makes that safe
    ///
    /// **A host's template output, never a request's.** The only callers are hosts rendering their
    /// own askama templates, which escape their own interpolations as every template here does — so
    /// what arrives is markup a host wrote, in the same sense the `{% block content %}` of any page
    /// in this crate is markup this crate wrote.
    ///
    /// **What must never reach it is a value from a request.** A host that formatted a form field
    /// into this string would be writing an injection with extra steps, and no amount of care in
    /// this crate could catch it. The rule is: render a template, pass the output; never build the
    /// string.
    pub content: String,
}

/// Which tab is showing.
///
/// **In bar order**, so the enum and `layout.html` can be read against each other. `Machine` leads
/// because it is the tab about the machine rather than about something put on one — and because
/// `km-admin` puts the same tab in the same place under the same name; see `Two admin surfaces, one
/// vocabulary` in docs/decisions/distribution.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The machine's name, its addresses and its password.
    Machine,
    /// The packages installed on this machine.
    Songs,
    /// The wallpaper rotation.
    Pictures,
    /// The SoundFont banks.
    Sound,
    /// Everything the machine found and could not use.
    Problems,
}

impl Tab {
    /// Whether this is the tab being drawn, for the `aria-current` mark.
    pub fn is(self, other: Tab) -> bool {
        self == other
    }
}

/// Which of the Machine tab's errands is open.
///
/// **In strip order**, the way [`Tab`] is in bar order, so the enum and `machine.html` read against
/// each other.
///
/// A save comes back to the pane it was made on, so this travels in a redirect's query string —
/// which is why it is an enum and not the string that arrived. `handlers::back_pane` is the only
/// way in, and the redirect is built from [`Self::id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// The three developer switches.
    Debug,
    /// What the machine is called.
    Name,
    /// Changing the password, and signing every phone out.
    Password,
    /// Whether the machine performs for itself.
    Demo,
    /// What the television speaks. The machine's own page only.
    Language,
}

impl Pane {
    /// Whether this is the pane being drawn, for the radio's `checked` mark.
    pub fn is(self, other: Pane) -> bool {
        self == other
    }

    /// The one word this pane is named by, in a radio's id, in its class and in a redirect.
    ///
    /// **One spelling for all three**, because the stylesheet's selector names the radio and the
    /// pane together: a class renamed without the id following is a section nothing can open.
    pub fn id(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Name => "name",
            Self::Password => "password",
            Self::Demo => "demo",
            Self::Language => "language",
        }
    }

    /// The pane a host actually draws, falling back to the one that is never empty.
    ///
    /// **`.settings .pane { display: none }` is unconditional**, which is the polarity argument
    /// `machine.html` makes: a browser that could not evaluate the selector shows every section
    /// rather than none. The consequence here is that a pane whose radio a host does not render
    /// leaves *no* pane open at all, and the settings half of the tab is simply gone — so a `pane`
    /// naming one is answered with Debugging rather than with nothing.
    ///
    /// One can arrive that way: `Language` has no route on a tool.
    pub fn drawn(self, capabilities: crate::machine::Capabilities) -> Self {
        match self {
            Self::Language if !capabilities.screen_language => Self::Debug,
            kept => kept,
        }
    }
}

/// What every page carries.
#[derive(Debug, Clone)]
pub struct Chrome {
    /// The tab showing.
    pub tab: Tab,
    /// The stamp on the stylesheet's URL. See `crate::ASSET_VERSION`.
    pub assets: &'static str,
    /// What the machine calls itself, for the heading.
    pub machine_name: String,
    /// What the browser tab says, machine name and all.
    ///
    /// Composed here because the title carries a value: `{{ machine_name }} — setup` leaves the one
    /// English word in the tab, and the tab is the half of a page a person sees while it is not the
    /// page they are looking at.
    pub title: String,
    /// The BCP-47 tag for `<html lang>`, from [`km_locale::Locale::tag`].
    ///
    /// **A page in Portuguese saying `lang="en"` is a page that lies to everything reading it but
    /// the person** — a screen reader picks the wrong voice, the browser offers to translate what
    /// is already translated, and hyphenation breaks. It is the one attribute here that has to
    /// follow the catalog.
    pub lang: &'static str,
    /// Whether this machine has a password.
    ///
    /// Drives the banner on every page, not just the Machine tab: `A machine with no password has
    /// no door` means every control here is open to the whole LAN until one is set, and a page that
    /// mentioned it only on the tab somebody might not visit would be hiding the thing that matters
    /// most about it.
    pub factory_password: bool,
    /// Whether debugging mode is on **this run**, for the wording on the Machine tab.
    pub debug_enabled: bool,
    /// Whether the settings file says debugging is on, and so what the switch changes.
    ///
    /// **The switch reads this and not [`debug_enabled`](Self::debug_enabled), which is the bug this
    /// field fixes.** The running value is a snapshot taken when the router was built and cannot
    /// move, so a switch drawn from it said *Turn debugging on* both before the press and after it.
    pub debug_stored: bool,
    /// Whether the development console's own switch is on, as settings hold it.
    pub dev_remote_stored: bool,
    /// Whether the console is actually being served — this switch and debugging both.
    pub dev_remote_served: bool,
    /// Whether the frame-statistics panel is on the machine's screen right now.
    ///
    /// No stored twin, because nothing is written down: see `Control::set_performance_overlay`.
    pub performance_overlay: bool,
    /// How many things are wrong with this machine, for the Problems tab's badge.
    ///
    /// On the chrome and therefore counted on **every** page, because the badge's whole job is to
    /// be seen from a tab somebody opened for another reason. The three reads behind it are cheap
    /// and are the ones the Pictures and Sound tabs already do per load — see `chrome`.
    pub problems: usize,
    /// Which program is drawing this page, where that is not the machine. See `Admin::program`.
    pub program: Option<&'static str>,
    /// What this host's surface can do, for the panes and entries the templates gate.
    ///
    /// On the chrome rather than passed to each page, because the tab strip is in the layout and
    /// every page extends it.
    pub capabilities: crate::machine::Capabilities,
    /// Whether this is a host's front door, which is drawn without the tab strip.
    ///
    /// **Every entry in that strip leads to a tab about a machine, and this is the page somebody is
    /// on before there is one.** A strip there is five ways to look at nothing, drawn over the one
    /// control that gets somebody out of it. See [`crate::Admin::door`], which is also where the
    /// reason the rest of these fields are empty here is written down.
    pub front_door: bool,
    /// The scripts this host asked for, in load order. See [`crate::Admin::scripts`].
    ///
    /// **On the chrome, so a host's scripts reach every page that host serves.** The upload forms
    /// are this crate's markup and appear on the machine's surface and a tool's alike, and an
    /// upload that reports nothing while it runs is a page that cannot be told from a hung one. A
    /// script is the only thing that can say a form post is still in flight.
    ///
    /// **The machine declares none**, so every page it serves is byte-for-byte scriptless and
    /// `no_page_the_machine_serves_carries_a_script` holds unchanged. What this gives up is the
    /// stronger claim that the tabs drawn here are script-free on *every* host.
    pub scripts: &'static [&'static str],
}

impl Chrome {
    /// The chrome a front door wears: a heading, a stylesheet, and nothing that needs a machine.
    ///
    /// **The heading names the program rather than the machine**, which is the one page where that
    /// is right: there is no machine yet, and `What the tool calls itself` requires a tool's pages to
    /// say so. [`Self::program`] is therefore `None` — the sub-heading would otherwise say the name
    /// twice.
    #[must_use]
    pub fn door(
        capabilities: crate::machine::Capabilities,
        program: Option<&'static str>,
        locale: Locale,
        scripts: &'static [&'static str],
    ) -> Self {
        let name = program.unwrap_or_default().to_owned();
        Self {
            tab: Tab::Machine,
            assets: crate::ASSET_VERSION,
            title: name.clone(),
            machine_name: name,
            lang: locale.tag(),
            factory_password: false,
            debug_enabled: false,
            debug_stored: false,
            dev_remote_stored: false,
            dev_remote_served: false,
            performance_overlay: false,
            problems: 0,
            program: None,
            capabilities,
            front_door: true,
            scripts,
        }
    }
}

/// Something that just happened, shown at the top of the page it happened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// `good`, `warn` or `bad`, which the stylesheet colors.
    pub kind: &'static str,
    /// What to say, worded for a person.
    pub text: String,
}

impl Notice {
    /// Something worked.
    pub fn good(text: impl Into<String>) -> Self {
        Self {
            kind: "good",
            text: text.into(),
        }
    }

    /// Something did not happen, and it is not a fault.
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            kind: "warn",
            text: text.into(),
        }
    }

    /// Something went wrong.
    pub fn bad(text: impl Into<String>) -> Self {
        Self {
            kind: "bad",
            text: text.into(),
        }
    }
}

/// One installed package, as the Songs tab shows it.
#[derive(Debug, Clone)]
pub struct PackageRow {
    /// Its id, which is also what a remove or a bank change names.
    pub id: String,
    /// Its title, or its id where it has none.
    pub title: String,
    /// Which build of it is installed. See [`crate::machine::Listing::version`].
    pub version: String,
    /// How many songs it holds.
    pub songs: usize,
    /// The block of a thousand its numbers sit in.
    pub bank: u16,
    /// Why this package's file is not the machine's to delete, or `None` if it is.
    ///
    /// **The row leaves the Remove control out entirely** when this is `Some`, and prints the
    /// sentence in its place — following the bundled bank in `sound.html`, whose comment gives the
    /// rule: a control that is always refused teaches somebody to ignore the row it is in. The
    /// wording is the machine's own, the very sentence an uninstall would have refused with.
    pub why_not_removable: Option<String>,
    /// What a screen reader announces for this row's Remove control.
    ///
    /// **Composed here rather than interpolated in the markup**, which is the rule the `|t` filter
    /// sets: a key goes in a template, and anything that puts a name inside a sentence is built
    /// where a test can reach it. `Remove` alone, repeated down a table, tells a screen reader
    /// nothing about which row it is on.
    pub remove_label: String,
    /// What a screen reader announces for this row's bank picker.
    pub bank_label: String,
    /// A label per flag the package carries, each drawn as a badge beside its title.
    ///
    /// Worded in [`crate::handlers`] through `flag_label`, so a flag this page has no word for is
    /// left out rather than printed as a code.
    pub flags: Vec<String>,
}

/// The songs tab.
#[derive(Template)]
#[template(path = "songs.html")]
pub struct SongsPage {
    /// The shared chrome.
    pub chrome: Chrome,
    /// What just happened, if anything.
    pub notice: Option<Notice>,
    /// What is installed.
    pub packages: Vec<PackageRow>,
    /// Every song on the machine and the packages holding them, as a sentence.
    ///
    /// Composed rather than two numbers with the nouns in the template: `{{ n }} songs in {{ m }}
    /// packages.` is three English words a catalog cannot reach, and it is two plurals — a machine
    /// with one package holding one song read `1 songs in 1 packages`.
    pub songs_count: String,
    /// What the file chooser offers, from `km_api::uploads::accept_for`.
    ///
    /// **A field rather than a literal in the markup**, and the name is `km-admin`'s so the two
    /// surfaces read alike. This template typed the list out until the function existed; see
    /// `accept_for`'s own note for why a typed list is a list that drifts.
    pub accepts: String,
}

/// The pictures tab.
#[derive(Template)]
#[template(path = "pictures.html")]
pub struct PicturesPage {
    /// The shared chrome.
    pub chrome: Chrome,
    /// What just happened, if anything.
    pub notice: Option<Notice>,
    /// How many pictures are in the rotation.
    pub count: usize,
    /// The one on screen.
    pub current: Option<String>,
    /// Why there are none, when there are none.
    pub problem: Option<String>,
    /// Whose pictures these are, in a sentence.
    ///
    /// **The sentence rather than the enum**, because the enum's three cases each need a different
    /// explanation and a template that branched three ways would put the wording in the markup.
    pub whose: String,
    /// Whether adding a picture would replace the set that is showing.
    ///
    /// The trap this page has to say out loud: `Where the owner's own wallpapers live` makes a
    /// non-empty owner folder win **outright** rather than merge, so the first upload silently
    /// removes the shipped pictures from the rotation. Deliberate, load-bearing, and exactly the
    /// kind of thing somebody should be told before they drop a file rather than after.
    pub first_upload_replaces: bool,
    /// How long each picture stays, as a sentence.
    ///
    /// **The unit is in the catalog rather than in the template**, because a bare `{{ secs }}
    /// seconds` puts an English word beside a translated label — which is what a Portuguese page
    /// read as `Muda a cada 30 seconds`. It is also where the plural lives, for the machine set to
    /// change every second.
    pub interval: String,
    /// The files in the folder, one row each.
    ///
    /// **Files, not pictures**, so a zip is one row saying how many it holds — the same unit a
    /// package is. Empty on a machine showing the bundled set, which is not the same as `count`
    /// being zero: there are pictures on the screen and none of them are the owner's to manage.
    pub pictures: Vec<PictureRow>,
    /// What the file chooser offers, from `km_api::uploads::accept_for`.
    ///
    /// **The one of the three that carries a media type.** `image/*` leads it so a phone opens its
    /// camera roll rather than its file browser, which matters here more than on the other two tabs
    /// because a photograph is the upload somebody most often has on a phone and nowhere else. The
    /// reasoning is in `accept_for`, not here, because it is the machine's rule and not this page's.
    pub accepts: String,
}

/// One file in the wallpaper folder, as the Pictures tab shows it.
#[derive(Debug, Clone)]
pub struct PictureRow {
    /// The slug a route names it by.
    pub id: String,
    /// The file name.
    pub name: String,
    /// How many pictures it puts in the rotation — above one only for an archive.
    pub images: usize,
    /// How large it is, already rendered — `1.3 MiB`.
    pub size: String,
    /// Why this file is not the machine's to delete, or `None` if it is.
    ///
    /// The same rule, and the same wording, as [`BankRow::why_not_removable`]: the row shows the
    /// sentence in place of the control rather than a control that is always refused.
    pub why_not_removable: Option<String>,
    /// What a screen reader announces for this row's Remove control. See `PackageRow`.
    pub remove_label: String,
}

/// One installed bank, as the Sound tab shows it.
#[derive(Debug, Clone)]
pub struct BankRow {
    /// The slug a route names it by.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Whether it is the one playing.
    pub current: bool,
    /// Whether it shipped with the machine.
    ///
    /// Drives the badge that says so, and nothing else: whether it can be removed is
    /// `why_not_removable`, which is not the same question — a `debug.soundfonts` slot is not
    /// bundled and is not removable either, so branching on this one offers it a button that refuses.
    pub bundled: bool,
    /// How large it is, already rendered — `261.9 MiB`.
    pub size: String,
    /// Why this bank's file is not the machine's to delete, or `None` if it is.
    ///
    /// The same rule, and the same wording, as [`PackageRow::why_not_removable`].
    pub why_not_removable: Option<String>,
    /// What a screen reader announces for this row's Remove control. See `PackageRow`.
    pub remove_label: String,
}

/// One output device, as the Sound tab offers it.
#[derive(Debug, Clone)]
pub struct OutputRow {
    /// The identifier `PUT /audio/output` takes. Opaque, and the whole of the setting.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Whether this is what the setting names.
    pub selected: bool,
    /// Whether it is plugged in.
    ///
    /// **Listed rather than dropped when false**, which the machine's own rule requires: a saved
    /// device unplugged for one evening is still the choice, and the setting is deliberately left
    /// alone — so a page that hid it would report the fallback as though somebody had chosen it.
    pub available: bool,
    /// Whether this row is the *follow the system* sentinel rather than a device.
    ///
    /// **Decided by the id and not by `AudioOutput::system_default`**, which is a different fact and
    /// was read as this one: that flag marks the real device the sentinel resolves to *today*, so
    /// trusting it labelled the onboard card "Follow the system" and left the sentinel showing the
    /// backend's untranslated English. See `km_api::machine::SYSTEM_OUTPUT`.
    pub is_system_choice: bool,
    /// Whether this is the device *follow the system* would pick today.
    ///
    /// Drawn as a note on the row, so choosing the sentinel is choosing something a reader can see.
    pub system_default: bool,
}

/// The level control on the Sound tab, where the active output has one.
///
/// **The reading and the control are separate fields here because they are separate acts on the
/// page**: `said` is always drawn, and the slider is behind a link.
#[derive(Debug, Clone)]
pub struct LevelControl {
    /// What it is set to, as a sentence naming the decibels.
    ///
    /// Composed in Rust following [`SoundPage::output_fell_back`]: it puts a number inside a
    /// sentence, which a template cannot do in a form a test can reach.
    pub said: String,
    /// The slider's current position, in hundredths of a decibel.
    pub db_centi: i32,
    /// The bottom of the slider, in hundredths of a decibel.
    ///
    /// **Not always the bottom of the control.** A control reaching −128 dB would put everything
    /// audible in the top sixth of the slider, so the slider stops at [`LEVEL_FLOOR_CENTI`] and the
    /// hint says the control goes lower.
    pub floor_centi: i32,
    /// The top of the slider, which is the top of the control.
    pub ceiling_centi: i32,
    /// The smallest move the slider offers.
    pub step_centi: i32,
    /// Whether the control reaches below the slider's floor.
    pub deeper_than_slider: bool,
    /// Whether the slider is being shown rather than the link that reveals it.
    pub open: bool,
}

/// How far down the slider goes, in hundredths of a decibel.
///
/// Below this is inaudible, and a control whose own floor is −128 dB would otherwise spend five
/// sixths of the slider on settings nobody can hear. The control still reaches its own floor, and a
/// level already set below this draws the slider from there instead.
pub const LEVEL_FLOOR_CENTI: i32 = -6000;

/// How large a rise asks first, in hundredths of a decibel.
///
/// Six decibels is a doubling of voltage. The accident worth stopping is a drag from one end of the
/// slider to the other, not a nudge — and a room balanced for a quiet source is loud at the top of
/// the range. Lowering never asks: too quiet is audible, costs nothing, and the same slider undoes
/// it.
pub const LEVEL_CONFIRM_RISE_CENTI: i32 = 600;

/// The sound tab.
#[derive(Template)]
#[template(path = "sound.html")]
pub struct SoundPage {
    /// The shared chrome.
    pub chrome: Chrome,
    /// What just happened, if anything.
    pub notice: Option<Notice>,
    /// The banks on this machine.
    pub banks: Vec<BankRow>,
    /// The outputs to offer, the sentinel first.
    ///
    /// **One row per physical output by default, with every spelling behind `?all=1`.** ALSA hands
    /// over its *configuration* rather than its hardware, so one jack arrives ten times under one
    /// string and the full list runs past thirty rows — see `Choosing the audio output device` in
    /// docs/decisions/, which requires both halves: a first screen somebody can read, and nothing
    /// hidden from the machine.
    pub outputs: Vec<OutputRow>,
    /// Whether every spelling is being shown.
    pub outputs_all: bool,
    /// What is actually sounding, as a sentence, or `None` when the machine reported nothing.
    ///
    /// **Drawn as well as the picker, because after a fallback the two disagree** — the setting names
    /// a device that is not there and the sound is coming out of something else. A page showing only
    /// the `<select>` would show the choice and not the consequence.
    pub output_playing: Option<String>,
    /// The sentence to print when the chosen device is absent, or `None` when it is not.
    ///
    /// Composed in Rust following [`PicturesPage::whose`]: it names a device, and a template that
    /// put a name inside a sentence would be markup a test cannot reach.
    pub output_fell_back: Option<String>,
    /// Whether the device can be changed right now.
    ///
    /// **The picker is drawn either way.**
    /// `A control that can only be refused is left out, not grayed` names `AudioOutputs::changeable`
    /// as the precedent for *having* the flag and not for spending it, because a device that cannot
    /// be changed now can be changed when the song ends.
    pub output_changeable: bool,
    /// The level the active output runs at, or `None` where it has none to set.
    ///
    /// **Left out rather than drawn dead when absent**, which is the other side of the bargain
    /// `output_changeable` makes: a device that cannot be changed now can be changed when the song
    /// ends, but an HDMI output hands the volume to a receiver and will never have a level. `A
    /// control that can only be refused is left out, not grayed` decides both.
    pub level: Option<LevelControl>,
    /// What the file chooser offers, from `km_api::uploads::accept_for`.
    ///
    /// See [`SongsPage::accepts`]. No media type on this one: `.sf2` is `audio/x-soundfont` at best
    /// and no picker maps that to anything.
    pub accepts: String,
}

/// One line of the confirmation's fact list — `Songs`, `120`.
///
/// (Placed here so the `MachinePage` fields above stay together.)
#[derive(Debug, Clone)]
pub struct Fact {
    /// What it is.
    pub label: String,
    /// What it says.
    pub value: String,
}

impl Fact {
    /// A fact, from anything that can be shown.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// The page that stands between a Remove control and a deleted file.
///
/// **One page for all three**, because a package, a bank and a picture differ only in their words —
/// which is what the `facts` list and `extra` are for, and why a zip's picture count arrives as a
/// `Fact` rather than as a field this struct would have to grow. The same
/// judgment [`PicturesPage::whose`] makes about composing a sentence in Rust rather than branching
/// three ways in markup.
///
/// **Only the three controls that destroy a file get one.** `static/admin.css` carries the rule and
/// the reason it is scoped: a page that asks twice teaches people to click twice, so moving a
/// package's bank, showing the next picture and setting a name all stay one click.
#[derive(Template)]
#[template(path = "confirm.html")]
pub struct ConfirmPage {
    /// The shared chrome. The tab it came from stays marked: this page is *inside* Songs or Sound.
    pub chrome: Chrome,
    /// Always `None` here, and it is not dead weight: `layout.html` renders a notice on every page
    /// that extends it, so leaving the field off would not compile.
    pub notice: Option<Notice>,
    /// The question, with the thing's name in it — `Remove “Volume 1”?`
    pub heading: String,
    /// What is about to go, in the `.facts` grid the stylesheet already has.
    pub facts: Vec<Fact>,
    /// The sentence that says a file is deleted and that nothing here undoes it.
    pub warning: String,
    /// A second consequence, where there is one — the bank being removed is the one playing.
    pub extra: Option<String>,
    /// Where the form posts: the same address this page was fetched from.
    pub action: String,
    /// The button's words. Not "OK": it says what it does.
    pub confirm: String,
    /// The tab to go back to, unchanged.
    pub back: String,
}

/// One package the machine found and refused, as the Problems tab shows it.
#[derive(Debug, Clone)]
pub struct RefusedRow {
    /// What a control names it by. See `km_api::machine::PackageProblem::id`.
    pub id: String,
    /// The file's own name.
    pub file: String,
    /// The folder it is in.
    ///
    /// **On the row, where every other surface deliberately shows only the name.** Two rows reading
    /// `carols.kmpkg` with the same reason and two different Delete links is worse than no page at
    /// all, and that is the observed case rather than a hypothetical one. `/admin/` is where this is
    /// publishable — it runs inside the machine and already prints paths in a refusal's sentence,
    /// which is the very distinction `A control that can only be refused is left out, not grayed`
    /// draws: the page prints the sentence, a remote gets the flag.
    pub folder: String,
    /// What a screen reader announces for this row's Delete control. See `PackageRow`.
    pub delete_label: String,
    /// What went wrong, in the machine's own words.
    pub reason: String,
    /// Why this file is not the machine's to delete, or `None` if it is.
    ///
    /// The same rule and the same wording as [`PackageRow::why_not_removable`]: the row prints the
    /// sentence in place of the control rather than offering one that is always refused.
    pub why_not_removable: Option<String>,
}

/// One option in a fault's own picker — a bank, or an output.
///
/// **Deliberately not [`BankRow`] or [`OutputRow`].** Those carry what their own tabs draw — sizes,
/// badges, refusal sentences — and none of it belongs in a fault row, whose whole job is to be one
/// control wide. One shape for both also means the template has one arm per fix rather than two.
#[derive(Debug, Clone)]
pub struct Choice {
    /// What the form posts.
    pub id: String,
    /// What to show, unless this is the sentinel.
    pub label: String,
    /// Whether it is the one in force.
    pub selected: bool,
    /// Whether this row is the *follow the system* sentinel rather than a named thing.
    ///
    /// **The label for it belongs in the template**, because that is where the `|t` filter is: the
    /// sentinel's name comes out of the audio backend in English, and the one place that can say it
    /// in the reader's own language is the markup. So this is a flag rather than a translated string
    /// composed in Rust, unlike every other sentence on this page — those take a *value*, and the
    /// filter takes none.
    pub is_system_choice: bool,
}

/// The control that answers a fault, where the fault has one.
///
/// **Every fault this page reports now has one, and that reverses a paragraph of the decision that
/// created the tab.** It said these rows carry no button because there is no file to delete, and
/// that what fixes them is a choice on another tab — which was right about the reason and wrong
/// about the remedy, because one of the three linked to a tab that had no control for it at all.
/// There has never been an output picker anywhere in the product until now.
#[derive(Debug, Clone)]
pub enum Fix {
    /// Choose a SoundFont bank. Posts to the same route the Sound tab's own buttons do.
    Bank(Vec<Choice>),
    /// Choose an output device. The same route and the same picker the Sound tab draws.
    Output(Vec<Choice>),
    /// Add a picture, which is the only thing that answers an empty rotation.
    ///
    /// No list, because there is nothing to choose between: the fault *is* that there are none. It
    /// posts to the Pictures tab's own upload route, body limit and all.
    Picture,
}

/// Something else about this machine that is not working.
#[derive(Debug, Clone)]
pub struct FaultRow {
    /// What it is about — `Sound`, `Pictures`.
    pub subject: &'static str,
    /// What is wrong, in a sentence the machine composed.
    pub text: String,
    /// The tab where the same thing can be done with more context around it.
    ///
    /// **Beside the control rather than instead of it.** The owning tab knows things this one does
    /// not — which banks are installed and how big they are, where the pictures come from — which is
    /// what the link is for, rather than for rescuing a row that has no control on it.
    pub tab: &'static str,
    /// The control that answers it.
    ///
    /// `None` is left possible on purpose rather than made impossible by the type: a future fault
    /// with no answer on this page should be reportable without inventing a control for it, and the
    /// row is still not a dead end because [`tab`](Self::tab) is not optional.
    pub fix: Option<Fix>,
}

/// The problems tab.
#[derive(Template)]
#[template(path = "problems.html")]
pub struct ProblemsPage {
    /// The shared chrome.
    pub chrome: Chrome,
    /// What just happened, if anything.
    pub notice: Option<Notice>,
    /// Packages that are on this machine and are not installed.
    pub refused: Vec<RefusedRow>,
    /// Everything else that is wrong, reported and not actionable here.
    pub faults: Vec<FaultRow>,
}

/// The machine tab.
#[derive(Template)]
#[template(path = "machine.html")]
pub struct MachinePage {
    /// The shared chrome.
    pub chrome: Chrome,
    /// What just happened, if anything.
    pub notice: Option<Notice>,
    /// Which errand is open. See [`Pane`].
    pub pane: Pane,
    /// Whether this host is holding a token the machine will accept.
    ///
    /// **Always `true` where [`crate::machine::Capabilities::choose_machine`] is off**, so the
    /// template's test never reads as a claim about a machine that has no such question: on its own
    /// page the caller is through the door or is not looking at the page at all.
    pub logged_in: bool,
    /// Every address this machine can be reached at.
    pub urls: Vec<String>,
    /// What to do with them, in the singular or the plural.
    ///
    /// Composed rather than a flat key: a machine with one interface offers one address, and
    /// *Open one of these* over a list of one is a sentence about a list that is not there.
    pub open_hint: String,
    /// How many songs it holds.
    pub songs: Option<usize>,
    /// The version it is running.
    pub version: String,
    /// The languages the television can speak, with the one it does marked.
    pub locales: Vec<LocaleChoice>,
    /// Who may do what on this machine, where the host can say.
    pub access: Option<km_api::dto::AccessDto>,
    /// Whether the machine is performing for itself when nobody is singing.
    pub demo_enabled: bool,
    /// Whether the settings file says so, and so whether it survives a restart.
    ///
    /// A separate fact from [`demo_enabled`](Self::demo_enabled) rather than a derived one: the API
    /// lets a run differ from what is written down, so a page that showed one number could not say
    /// *on tonight, off tomorrow* — which is the state the `persist` box exists to produce.
    pub demo_stored: bool,
    /// Seconds of quiet before the machine performs for itself.
    ///
    /// **A box on its own form rather than a third field on the switch's**, and the reason is the
    /// `persist` box two lines up: that form is *tonight or for good* and this number is neither —
    /// it is always written down. A single Save over both would have to answer what `persist` meant
    /// for a field that ignores it. See `Controller::set_demo_delay`.
    pub demo_delay_secs: u32,
    /// The largest number the box will take, for the `max` attribute.
    ///
    /// Drawn from `km_api::machine::MAX_DEMO_DELAY_SECS` rather than typed into the template, so the
    /// browser refuses exactly what the route refuses and nobody has to discover the cap by being
    /// told no.
    pub demo_delay_max: u32,
    /// Whether this host can be shut down and restarted, and so whether the card exists.
    ///
    /// **Absent rather than disabled**, which is this page's standing rule: a grayed control invites
    /// somebody to work out what would enable it, and the answer here — *run on a supervised
    /// appliance* — is not something anybody can do from this page. On a desktop or a phone the
    /// section is simply not part of the document.
    pub power: bool,
}

/// The last page a browser gets before the machine stops answering it.
///
/// **A page rather than the redirect every other control here ends with**, and the two cases are why:
/// a redirect after *Restart* is a guaranteed failed load, because the machine is down for the two
/// seconds the browser spends following it; a redirect after *Shut down* is a spinner and then a
/// connection error. Both are honest and both look like the control broke. This says what happened
/// instead, and — for a restart, where there is something to come back to — brings the tab back by
/// itself.
#[derive(Template)]
#[template(path = "farewell.html")]
pub struct FarewellPage {
    /// The shared chrome. The Machine tab stays marked: this is where the control was.
    pub chrome: Chrome,
    /// Always `None`. `layout.html` renders a notice on every page that extends it, so leaving the
    /// field off would not compile — the same note [`ConfirmPage::notice`] carries.
    pub notice: Option<Notice>,
    /// What is happening, in two words.
    pub heading: String,
    /// What it means for whoever is holding the phone.
    pub said: String,
    /// Seconds before the browser returns to the Machine tab, where there is one to return to.
    ///
    /// `None` after a shutdown, because a page that kept retrying a box which is off would spend
    /// the evening showing a connection error. A plain `<meta http-equiv="refresh">` rather than a
    /// script, like everything else on these pages.
    pub refresh_in: Option<u32>,
}

/// One option in the language picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleChoice {
    /// The BCP 47 tag, which is what the form posts.
    pub tag: &'static str,
    /// What this language calls itself.
    ///
    /// **Not translated, and that is the point** — see `Locale::endonym`.
    pub name: &'static str,
    /// Whether it is the one in use.
    ///
    /// **Which of two languages that is depends on the picker.** The machine's *Screen language*
    /// pane marks what the television draws in; `km-admin`'s front door marks what its own pages
    /// are in. One view model because the rule a picker keeps is the same either way.
    pub chosen: bool,
}

impl LocaleChoice {
    /// Every language, with the one in use marked.
    #[must_use]
    pub fn all(current: km_locale::Locale) -> Vec<Self> {
        km_locale::Locale::ALL
            .iter()
            .map(|locale| Self {
                tag: locale.tag(),
                name: locale.endonym(),
                chosen: *locale == current,
            })
            .collect()
    }
}

/// The login page.
#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    /// The stamp on the stylesheet's URL.
    pub assets: &'static str,
    /// What went wrong last time, if anything.
    pub notice: Option<Notice>,
}
