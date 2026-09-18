# The API and the network

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## The URL prefix is the permission

**One shared admin password, and everything it guards lives under `/api/v1/admin/`.** A path under
that prefix demands a valid token; a path outside it never does. There is nothing to configure and
nothing an owner can get wrong.

**There is no `public`/`admin` map in `settings.json`.** A table of route ids is free to drift out of
step with the router it describes, and the freedom forty-six entries bought — closing
`packages.install` on one machine — is one nobody ever used.

**A reader can see the permission in the URL, and a test cannot be written that does not check it.**
`routes::needs_admin_token` is four lines over the request path, `ApiState::authorize` is its only
caller, and a route added under `/api/v1/admin/` is gated the day it is written rather than when
somebody remembers to add a row. `the_admin_prefix_is_exactly_what_needs_a_token` sweeps the whole
surface and asserts both directions of that.

**The prefix test carries its trailing slash.** `"/api/v1/admin"` alone
matches `/api/v1/adminfoo` — and, far worse in the other direction, would silently protect a future
`/api/v1/administration` under a rule nobody had written down. A unit test pins all three spellings.

**The cost is a resource split across two prefixes**: `GET /api/v1/demo` and
`PUT /api/v1/admin/demo` are the same thing filed in two places, and so are the audio, package and
wallpaper pairs. A REST purist would put them together and reach for a table to say which verb is
privileged — which is the table this rule does without.

**One exception, and it is the way in.** `POST /api/v1/admin/login` is under the prefix and needs no
token, because gating the login behind a token would make the password unusable. It is defended by
the rate limiter instead.

**There is no `settings.transpose` or `settings.melody`, and there cannot be.** A key change is
the same permission as any other setting — a performance knob a singer reaches for — so it sits
outside `/admin/` with search and queueing, permanently.

## Network reach

**The machine binds `0.0.0.0:8177` out of the box.** A default reaches only an install with no
settings file; one that has a file keeps what it says.

A karaoke machine whose remote nobody can connect to is the broken case, not the safe one — mDNS, the
QR code and the connect panel all exist to serve a phone, and every one of them is inert behind a
loopback default.

Paired with a password the machine gives itself and a fixed set of admin routes, this is a judgment
about a **home LAN**: anybody in the room being able to queue a song is the design, and everything
that reconfigures the machine is behind a code on its own screen.

**It stops being right the moment port 8177 is reachable from outside one**, and what changes then is
not a list of routes — it is the password. A generated PIN is fine for a room and is not a secret
from the internet, so an owner forwarding the port should set one of their own first, which is what
the banner on `/admin/` says until they do. `bind: "127.0.0.1:8177"` shuts it back in entirely.

**There is no list of routes to move first.** `packages.install`, `packages.uninstall` and the debug
routes are already either behind the prefix or behind the debugging switch. See
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission) and
[`A machine gives itself a password, and shows it on the television`](#a-machine-gives-itself-a-password-and-shows-it-on-the-television).

## The machine holds its port, rather than claiming it once

**A listening socket is taken again whenever it stops working.** A platform that suspends an
application destroys its socket while the application is away, and an interface going down takes one
with it. What comes back from either has a screen, a catalog and a synthesizer, and no way for
anybody in the room to reach it.

**Nothing gives up, and the port never moves.** A machine that stopped trying is one somebody has to
notice and restart, and the difficulty is precisely that nobody was watching when the socket went.
The address bound again is the one already held rather than the one asked for, because by then it is
on the television, in a QR code and in whatever a phone remembered. A machine that came back on a
different port would be a machine nothing can find, which is the fault it was recovering from.

**Coming back to the screen is a reason to take the port again, and the machine does not check
first.** What a suspended application is handed back can be a descriptor that neither accepts a
connection nor reports a failure, and that is indistinguishable from a network nobody is using. Only
the host knows it has returned, so it says so. Proving a socket still works costs a connection to it
and answers for that instant alone, where replacing it is a bind.

**While there is no socket, the screen says so and no address is published.** The connect panel names
the reason in place of a URL, which is the same sentence a port that was already in use puts there. A
machine claiming an address that answers nothing is worse than one admitting it has none: every
remote believes the claim, and the person standing in front of it has nothing to go on.

## Debugging is a mode, and the machine says when it is on

**`debug.enabled` governs the whole `debug.` section of settings, and while it is off the two
`debug/play-*` routes are not mounted at all.** Off means a 404 — genuinely not there — rather than
a refusal, and `play_file_roots`, `packages`, `wallpapers`, `soundfonts` and `soundfont_slot` are
all ignored with it.

**One switch, not one per route.** Every route that writes to disk is behind `/api/v1/admin/`, a
password always exists, and the package builder can hold one, so a narrow switch over the upload
route alone would have nothing left to do.

**One switch over five settings rather than five that each mean something slightly different.**
`debug.` is the one place `settings.json` may name an individual file — the standing decision
`Only debug. names a file` — which makes its entries exactly the ones that point the machine at
arbitrary paths. An owner deciding about them one at a time is an owner deciding five times about
the same question.

**A populated section with the switch off is logged, not silently ignored.** One `warn` at startup
naming each field it is passing over. Dropping a value without saying so is the trap: an owner whose
machine quietly stops honouring `debug.soundfonts` concludes the soundfont switcher broke rather
than that a switch they have never heard of is off.

**The two play routes are public when they exist, and are deliberately not admin routes.** A third
state — mounted but password-gated — would put a token-demanding path outside `/api/v1/admin/`, and
the URL would stop being the permission. On means open; off means absent; there is no middle.

**`PUT /api/v1/admin/debug` is what moves it, and `GET /api/v1/debug` reports it.** Turning it on is
an owner's act, because what it opens is a route that plays any file the machine can read; asking
whether it is on is a curation tool's business before it offers a Play button, so that half is
public. The same split `/demo` makes one screen over.

**It takes effect at the next start, and every surface says so.** The routes are mounted when the
router is built rather than checked per request, which is what lets an unmounted route answer 404
instead of carrying a disabled handler. The settings half takes effect at once. That asymmetry is
the price of the clearer surface.

**A debug build has it on and a shipped one does not**, through a three-state `Option`. An owner's
`true` or `false` always wins; **absent** means nobody has said and follows the build. In a checkout
the person running `cargo run` is the owner; on Android there is no command line at all and
`settings.json` sits in app-private storage reachable only through `adb`. The `Option` is
load-bearing: settings are saved by serializing the whole struct, so a plain `bool` defaulting to
the build would write `true` into a device's file on its first debug run and leave it there when the
same device took a release APK over the top.

**Three screens carry the switch, and that is not duplication.** The owner's page at `/admin/`,
because it is the machine's own configuration surface; `km-admin` and `km-package-builder`, because
a curator meets the closed door there and both hold the password. The dev remote carries it too, and
reaching *that* means starting the machine with `--dev-remote` — see
[`Dev remote in release builds`](remotes.md#dev-remote-in-release-builds).

**Discovery says whether a machine is in the mode, and a tool asks before it sends.** Refusing *after*
an upload does not reliably refuse at all: a server that answers mid-request and closes leaves the
sending half looking like a dropped connection, so the most useful message — the one naming the
setting to change — arrives as "the machine is not answering", if it arrives. With a four-byte fixture
that looks like a **flaky test**; with a video it is the normal outcome. `debug_enabled` on
`GET /discover` costs one cheap round trip on a call the tool already makes.

**The same reasoning applies to `packages.upload`.** Making it an admin route puts a 401 on a
multipart stream, which is the same failure with a different cause — so
`km-package-builder` checks its own token before sending a byte, rather than discovering the refusal
a gigabyte later.

**`km-admin` does the same on all three of its sends, and it is the tool that needed it most.** Its
files arrive from a browser, so a send with no password behind it would have the *page* push two
gibibytes across loopback and this program stage them to disk, to be refused by a machine that could
have said no before any of it moved. The check is in two places on purpose: early in the request
handler, which is what saves the transfer, and as a floor in the client itself, which is what covers
a call site that forgot. Two do exactly that — sending a bank this program downloaded and a
wallpaper pack it built — and neither goes through the handler.

**The refusal names a tab rather than restating the wire's words.** Songs, Pictures and Sound are
exactly the three pages with no password box on them, so *"this machine has a password and this
program has not been given it"* is true and useless there; it says to type it on the Machine tab.
None of this covers a token that has **expired**, which still fails on the wire — the stale token is
dropped there and the next page draw asks again.

**Nothing staged for an audition outlives the song that played it.** Not the *run* — an audition is
never cataloged and `play_audition` only ever resolves the folder of the upload in hand, so **it can
never be played again**: the moment it stops being the loaded song it is a gigabyte of scratch nobody
will read. On a television that folder is app-private storage with no other reclamation at all. So the
folder goes when the song is displaced, at the two places `state.loaded` is written, which between
them cover a song ending, Stop, Skip, the next queue entry, a demo and a second upload.

**It is tried at once and looked at again only if something refused**, rather than deferred on a
timer. The engine does not hold the file — `VideoSong` and `CdgSong` own the reader and dropping one
joins the decoder thread — so the drop at displacement is synchronous, and the only holder left is the
display thread's per-frame `Arc`, which lasts one drawn frame. Eight looks at 250 ms is two seconds,
after which the holder is an antivirus scan or an `adb pull` rather than anything this process owns,
and the warning says so once instead of scanning the folder every quarter second all evening.

**Two folders are protected deliberately**: the song playing cannot be deleted, and the upload still
arriving must not be — a gigabyte over Wi-Fi is minutes, so a first song ending mid-upload would
otherwise take the folder being written into. That second one is the only thing about an audition the
machine remembers between requests, and it has to be, because no later request can tell a folder being
written into from an abandoned one. Two curators uploading at once still leaves the older staging
unprotected, which is accepted: this is a single-curator route.

**The purge at startup and the sweep on the way in are backstops rather than the mechanism.**
A `SIGKILL`, a power cut or an Android low-memory kill reaches no hook, so the whole folder goes at the
next start, when nothing can be open. A clean stop purges too, which reclaims the space immediately on
Linux and Android — where unlinking an open file succeeds — and falls back to the startup purge on
Windows.

## The development console has an API that needs no password

**The whole API is mounted a second time at `/dev/api/v1/`, and nothing under that prefix asks for a
token — including the paths filed under `/admin/`.** It exists only while **two** switches are on:
`debug.enabled` and `api.serve_dev_remote`. `--dev-remote` turns on both for one run.

**What forced this was the permission moving, not the console changing.**
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission) put every write under
`/api/v1/admin/`, and nine of `/dev/`'s calls are writes — so a page whose entire value is being
unpolished and immediate acquired a password box you had to fill in before the interesting half of
it worked. Worse, it had already been broken twice by routes moving out from under it, with nothing
checking the paths it names. A surface you use to find out whether the
API is complete is the one surface that must not be the last to hear about a change to it.

**It follows the prefix rule rather than carving an exception out of it**, and that is the property
worth having. `/dev/api/v1/admin/password` does not begin with `/api/v1/admin`, so
`needs_admin_token` already misses it — there is no second rule, no allow-list, and nothing for the
one middleware to special-case. The tests name `DEV_API_PREFIX` anyway and sweep the whole of
`SURFACE` under it, because behaviour resting on the shape of a `strip_prefix` is behaviour a later
edit can close in silence: loosening that test to a `contains` would shut the console with every
other test still green.

**Two switches instead of a password, and the second one is the decision.** Requiring
`debug.enabled` as well means a passwordless copy of the API is never one tick away from a machine
in a living room — and it lines the console up behind the switch this product already treats as *this
machine is being worked on*, the one `/discover` reports and both admin surfaces warn about. It also
means the console cannot be reached on a machine whose owner has not also accepted the debug routes,
which is the same class of thing.

**The cost is real and is the price.** With both switches on, anyone who can reach the machine can
change its password, delete its packages and end every session, with no credential at all. Three
things bound that rather than one: the pair is off in every build and in `ApiConfig::default()`, the
machine draws a marker on its own television for as long as it is true (see
[`The machine says on its own screen when it is in developer mode`](interface.md#the-machine-says-on-its-own-screen-when-it-is-in-developer-mode)),
and every surface carrying the switch says in as many words what it opens. **A confirmation page was
considered and rejected**: this is a switch for somebody who is working on the machine, and a page
that asks twice teaches people to click twice — the rule
[`A page asks before it deletes a file; the API does not`](#a-page-asks-before-it-deletes-a-file-the-api-does-not)
already scopes to files, and no file dies here.

**`GET /api/v1/dev-remote` and `PUT /api/v1/admin/dev-remote` are the switch**, the same public-read
and admin-write split `/debug` and `/demo` make. The reply carries **two** fields — `enabled` is
this switch's own position and `served` is whether anything is actually up — because one of them
explains nothing: an owner who ticks the box and finds `/dev/` answering 404 needs to be told that
debugging is the missing half, and a route reporting only its own state cannot tell them.

**`DebugDto` gained `stored` beside `enabled` in the same change, and that was a bug rather than a
feature.** Both switches take effect at the next start, and the running value is a snapshot taken
when the router was built — so a page drawn from it showed *Turn debugging on* both before the press
and after it, and a reply echoing the request told a caller the surface had closed while it was
still open. `enabled` is what is running and `stored` is what the next start will do; they disagree
for exactly as long as it takes to restart, and every surface says so while they do.

**`/dev/api/{*rest}` is mounted whether or not the mirror is.** With the console off, that path had
nothing above it and fell through to the root fallback — so a JSON client got the HTML landing page
with a 200 and tried to parse it. Not hypothetical: the console's own address box lets somebody
point it at a machine whose console is off, and then *every* call did that. It is the same trap
`/api/{*rest}` already existed to close, and the fix is the same 404.

**The page keeps its address box and loses its password box.** Pointed at a machine whose console is
off it now gets an honest 404 per call, which its own log pane already shows plainly, rather than a
401 it has no way to answer.

## The ports are adjacent, and the order is the order they arrived

**8177 the machine, 8178 the package builder, 8179 the offline remote, 8180 KaraokeMachine Admin.**
Each new program takes the next number, and the constant lives in the crate that serves on it rather
than in a table all four read.

**Adjacency is the whole feature.** All four can run on one desk at once, and what somebody has in
front of them is a URL in a browser's address bar. `:8180` answering "which of these am I looking at?"
without a lookup is worth more than any scheme that grouped them by kind.

**Sequential rather than reserved.** The alternative is picking a block and defending it, and the only
thing that would buy is the freedom to insert a program between two others — which would immediately
break the property above.

Three of the four bind **loopback** unless asked otherwise; the machine is the exception, because a
karaoke machine no phone can reach is not one. `km-admin`'s `--lan` prints a warning when used, since
that program has no password and holds whatever API keys it has been given.

## A second machine on one box

**`--api-bind` moves the API for one run and is never written down.** Every other way to move it says
where the machine lives from *now on*, which is right for the appliance and wrong for two copies
running side by side on a development box, where the second must not quietly rewrite the first one's
home. So the flag reaches `ApiConfig` and never `Settings`.

A bare `--api-bind 8277` keeps whatever interface `api.bind` names and moves only the port; a full
`--api-bind 127.0.0.1:8277` says both, and **on Windows that spelling is the one to prefer** — a
listener on `0.0.0.0` raises a firewall prompt per program and port and a loopback one raises none.

**Pair it with `--data-dir`, or it is half a separation**: two machines on two ports still sharing one
catalog, one packages folder and one settings file are not two machines.

**`--data-dir` moves the data and nothing else** — settings, catalog and packages, never the assets.
Assets ship with the build rather than accumulating with use, so the command line takes
`Paths::data_rooted_at`, which leaves asset discovery exactly as a run with no flag would find it.
Moving them too gives a second machine with no SoundFont, no font and no wallpapers, coming up on a
sine test tone over a plain gradient.

A value that will not read is refused rather than defaulted, because the value it would default to is
the port the flag was typed to avoid — and the failure would present as the *first* machine losing its
remote.

## Discovery

**mDNS/DNS-SD advert**, a public `/api/v1/discover` endpoint, and the URL plus a **QR code on
screen**.

## The advert names the address the machine chose

**A `url` TXT record carrying the machine's own preferred URL, and every browsing client prefers it to
guessing.** The advert still publishes every reachable address as an A record; what is added is that
the machine says which one it means.

**The ranking does not survive the trip.** `km_api::connect` picks the address to show by **interface
name** — that is how a Hyper-V switch, a WSL adapter or a VirtualBox host-only network gets demoted
below the Wi-Fi card — and an A record is a bare number with no name attached. A browsing client is
left with `rank_of_address`, which knows only which private range an address is in and cannot separate
two adapters inside one.

**And a client does not see the whole set either.** `mdns-sd` sends only the addresses on the subnet
of the interface a packet leaves by, and resolves a service as soon as one address has landed. A
machine holding four addresses announces them one per packet, and a client that reads the first
announcement and stops is not choosing badly — it is not choosing at all. `browse` merges every
announcement of a machine, which is right regardless; the TXT record is what makes it unnecessary to
wait for the merge to be complete.

**The record is shape-checked and not otherwise trusted**: `http://<IPv4>:<port>`, nothing else. A
hostname would hand the choice to DNS and another scheme or port would let an advertisement point a
client off the machine entirely. It deliberately does **not** have to be one of the announced
addresses — a client on one subnet legitimately never receives the A record for the address the
machine picked. Anybody who can write this TXT record can already write an A record, so the check
exists to stop that reach widening, not to close it.

The cost is one obligation on the advertiser: a change of *preferred* address is a change worth
re-registering for. `advert_action` compares the first entry in place and the rest as a set, so the
anti-flap property is kept exactly where it was earned.

## A machine is known by its id, and its address is a cache

**Every client anchors on the machine's instance id and treats the address as something it can be
wrong about.** A URL is the one thing about a machine on a home network that does not hold still: the
router hands the lease to something else overnight, and the address a device woke up holding is a
printer or nothing at all while the machine itself is on, announcing itself, and unreachable to the
only client that wanted it.

**The identity is the instance id.** `new_instance_id` mints eight random bytes on first run,
`machine.instance_id` persists them, and they go out as the `id` TXT record and in `/discover`.

**Eight random bytes and not a `uuid` crate.** Sixty-four random bits minted once and never changed
*is* what a UUID would be here, and this one is already published, already persisted and already
parsed by three programs.

**The record is one type in `km_api::discover::known`, and the storage is deliberately not shared.**
`Known` carries the id, the address, the name and when the machine last answered; the remote and
`km-admin` keep one as JSON in their data directories, and the package builder keeps the same fields
in the *workspace* database — because that address is corpus-scoped, so a second computer opening the
same corpus talks to the same machine. **What is shared is the type and the policy, not the file.**

**The id is optional.** A record with none behaves as a bare address, which is what a machine pinned
by hand has until `/discover` answers and fills it in.

**A device that knows which machine is its own may connect to that one and to no other.** Until it
knows one it takes what the network offers, which is how a remote opened for the first time finds the
machine in the room; from the moment something has answered and said what it is, the only address it
will move to on its own is that machine's. A machine switched off for the evening and a remote carried
to another house are the same situation seen twice, and in both the one thing a device must not do is
quietly go and live on somebody else's box — with their catalog on screen and no sign of why.

**`anchored` is that question in one word, and the id is what answers it.** A record carrying none
names an address and no machine, so it anchors nothing and behaves as a bare address exactly as it
always has. `adopts` answers only what is left: on a device that knows of no machine, a remote opens
on whatever is in the room and a tool waits to be pointed at something.

**Moving to a different machine is a person's act.** Typing an address, or pressing a machine on the
list a look turned up — the offline remote's *Rescan* offers rather than takes for the same reason
`km-admin` and the package builder list rather than set.

**`choose` is one pure function and there is no second one.** A pin beats everything, because somebody
naming an address is giving an instruction about an address. Otherwise the remembered id, seen
announcing itself somewhere else, is where that machine is — and the interesting split is over what is
answering at the *old* address: a **different** id means the address now belongs to somebody else,
which is the overnight-DHCP case and the one only an identity can see; the **same** id means one
machine reachable two ways, so nothing moves; and nothing having said yet means stay while the record
is fresh and move while it is stale.

**Staleness changes eagerness and never correctness.** `STALE_AFTER` is six hours, and the number
matters less than the shape of being wrong about it: the event that moves an address is not a lease
expiring on a schedule but a machine switched off overnight, so six hours is longer than an evening's
use and shorter than a night. Crossing it too eagerly costs one comparison against a registry that is
already current; too late costs an evening on an address nothing answers. **The remembered address is
used immediately either way** — opening instantly beats opening in a second and a half.

**A machine answering at a *remembered* address does not get to become the machine.** A remote
holding a record for machine X, opening on the address it last saw X at, and meeting machine Y there
would overwrite its own record with Y before `choose` ever ran — and `choose` would then look for Y,
find Y exactly where it was, and correctly conclude nothing had moved. **The one case an identity
exists to catch would be the one case it could not catch**, and nothing in the test suite catches
it.

**The distinction is who chose the address.** `asked for` is somebody saying *that one* and `found on
the network` is the network offering this machine, so in both what answers is legitimately the machine.
`remembered` is neither — it is the device's own guess from yesterday, and a guess may not rewrite the
fact it was a guess about. A record with no id yet is not anchored to anything and always takes what
answers, which is how a first connection learns its machine at all.

The cost is one bounded case: a machine reinstalled at the same address gets a new id and this remote
keeps a dead one, goes on working because the address is still right, and adopts the new identity the
moment somebody presses Rescan or types the address.

**A record that has never connected counts as stale**, which is the honest reading of an address of
unknown age. The timestamp is refreshed at most once an hour while a connection holds: writing on
every refresh would be a write a second for an evening, and writing *only* when the address changed
would freeze it at the first connection, so a machine in continuous use would read as stale six hours
later.

## Discovery listens, rather than being asked

**One mDNS daemon is held open for the life of the process and a registry answers from what it has
heard.** Opening a daemon, draining events for a fixed window and shutting it down again means nobody
is listening between two of those, every caller pays the timeout on every look, and none of them ever
learns anything in between.

**`ServiceRemoved` is believed and silence is not**, which is the rule most likely to read as a leak.
A departure marks a row absent and removes nothing. The reason is Android: the multicast lock is held
only between `onStart` and `onStop`, so a remote in somebody's pocket hears nothing and `mdns-sd` duly
expires a machine that is switched on and two meters away. Dropping the row would turn *the phone was
in a pocket* into *the machine went away*. So **absence is never evidence**: an absent row stays a
usable cache of an address and may never *cause* a move, and `choose` requires a present sighting in
every branch.

**Looking again means building a new daemon, not asking the old one harder.** A poke — a button, a
resumed application, a connection that has just dropped — throws the `ServiceDaemon` away and starts
another, keeping the registry. **Two cheaper things do not work.** Calling `browse` a second time
*destroys* the watcher, because `mdns-sd` keeps one listener per service type and overwrites it, so
the second call closes the channel the reading thread is on and discovery freezes silently for the
life of the process. `verify` — RFC 6762 §10.4 record verification, which is the right question —
does not help either, because a daemon binds its sockets per interface when it starts and on a phone
that is exactly what has gone stale. **A socket that is no longer receiving cannot be asked more
politely.**

**The registry is a type apart from the daemon, because only one of them can be tested.** Opening a
daemon binds `0.0.0.0:5353`, which `CONTRIBUTING.md`'s *No test binds a non-loopback address* forbids
— and a watcher is worse than a one-shot, since it goes on retransmitting rather than shutting down
after a timeout. So `Registry` holds the merge and has no socket in it, every test drives one by hand,
and nothing constructs a `Watcher`.

**Matching a departure to a row costs one field.** A `ServiceRemoved` carries only
`(service_type, fullname)`, so a departure has nothing to match on unless the fullname is kept. It is
kept as a *list*, because renaming a machine re-registers it under a new label while its id stays put,
and both name it until the old one is withdrawn.

**Time is kept in two clocks on purpose.** The registry measures in `Instant` — *how long since this
process heard from it*, monotonic and meaningless across a restart — and the stored record in
`SystemTime`, because it has to survive being written to a file. Mixing them would make every record
read as fresh after a reboot.

## `KM_NO_MDNS` declines the multicast socket, and one function honours it

**A checkout can be told to open no mDNS daemon, and nothing an owner installs is told.**
`mdns_sd::ServiceDaemon::new` binds UDP `0.0.0.0:5353` and `[::]:5353` on every interface, and
Windows treats that exactly as it treats a listening TCP socket: a dialog, and a rule keyed on the
full image path. A tree being worked on relinks constantly and holds a worktree per branch, so every
rule such a prompt writes names a path that will never exist again — the dialog names a build hash
rather than a program, and answering it buys nothing.

**Binding the HTTP listener to loopback does not answer this**, which is what makes the variable
worth having. The curation tool, the offline remote and the picture-and-bank tool all bind
`127.0.0.1` unless asked for `--lan`, and all three browse for machines regardless; the socket that
asks is the multicast one, and it has no address to move.

**One function in `km_api::discover` opens every daemon in this repository**, and it answers `None`
when the variable is set. The browsing watcher, the one-shot browse and the advertisement all go
through it, so a program inherits the switch without a flag of its own and a test inherits it without
knowing it exists. A switch honoured in three places out of four is not a switch, so
`tools/dev/check-mdns.sh` asserts the function is the only caller.

**A declined daemon and a daemon that would not start are one state**, and the honest one: a network
with multicast blocked answers an empty list, and so does this. `Watcher` already treats "no daemon"
as ordinary and lets a later rescan try again, so the switch adds no state to it — and a locator that
cannot browse says so through `can_browse`, rather than offering a *Rescan* button that can only ever
report nothing.

**Set and not `0` is yes; absent or `0` is no** — the rule `KM_LOG_FILE` and `KM_FRAME_STATS` carry,
because a third boolean with a rule of its own is what makes all three unguessable. The `0` spelling
is what lets one command ask for mDNS where the variable is set for everything around it, which is
the case in a shell with no way to unset a variable for a single process.

**`.cargo/config.toml` sets it, and that is the line between a checkout and an install.** A release
runs from a package or a staged build and never through cargo, so a machine still advertises itself
and a remote still finds it with nothing typed. `Taskfile.yml` sets it for what it drives and clears
it for the two tasks that launch a staged build, because running one of those is how somebody looks
at the shipped behaviour.

## A machine has a name its owner chose, and one way in to set it

**`PUT /api/v1/machine/name` and `--set-name`, and the name takes effect without a restart.**
`machine.name` reaches the DNS-SD instance label, the `name` TXT record and `GET /discover`; without a
route, setting it means editing JSON, which on an appliance under a television means `ssh` or nothing.

**A route of its own, not a `SettingsPatchDto` field.** The settings route carries the knobs that
belong to a *performance* and ride the 4 Hz state broadcast, and a name is installation configuration
that does neither. The output device, the debugging switch and demo mode each took a route of their
own for the same reason.

**`PUT /api/v1/admin/machine/name`, so it needs the password.** It changes what the box is called on
somebody else's network and every phone in the house sees the result.

**No `GET` twin.** `/discover` is public, always, and carries the name —
which is the entire point of a name.

**Sixty-three bytes, because a DNS-SD instance name is one DNS label.** The cap is not a preference
and cannot be relaxed without the advert failing on the network, where nothing would report it. It
binds a second time through `Discovery::txt_records`, whose test pins a 400-byte ceiling so the record
set fits one packet. The cut walks back to a character boundary: "Karaokê da Sala" is not fifteen
bytes, and slicing a multi-byte name at 63 would panic rather than shorten it.

**A blank name is refused on the way in and tolerated on the way out.** `tidy_name` is the writer's
rule and returns nothing for a name that is only whitespace, so an empty name is never stored —
otherwise every client would separately have to decide what one meant. `display_name` is the reader's
rule, because refusing on the way in cannot stop a hand-edited settings file from
putting one on the wire; a client with no name shows the address rather than an empty heading.
`KaraokeMachine` is deliberately **not** filtered out there: it is a real answer for somebody who has
one machine.

There is no `--clear-name`. The way back is `--set-name KaraokeMachine`.

**Written down before it is answered to.** The controller persists and only then does the running
machine start using the new name, so a failed write leaves the machine and its settings file agreeing.
Saved immediately rather than at shutdown: a shutdown hook is exactly what a power cut does not run,
and a machine that came back under its old name would look like a rename that silently failed.

**The advert re-registers on a rename.** Comparing addresses only answers `Keep`, so `settings.json`
and `/discover` both change at once while every phone on the network goes on showing the old name
until the process restarts. The name is compared **in place**, beside the first address, because it is
published twice and a rename is a change of published fact rather than a reordering. The anti-flap
rule for the tail of the address list is untouched.

## A machine gives itself a password, and shows it on the television

**Every machine has an admin password from its first start.** It generates a six-digit PIN, hashes it
into `settings.json`, keeps the plain text beside the hash, and draws it on the connect panel next to
the address and the QR code. Nobody sets a first password, because there is never a machine without
one.

**There is no first password to set, only one to change**, and the page that changes it is behind the
password it is changing. A machine that started passwordless would be an open one, and the act that
closes it cannot be offered on the open page or the first stranger to find the address claims the box
— a knot that only ever came apart by there being no open state to start from. `--set-password` on a
desktop and `POST /api/v1/admin/password` from something that already knew the address are neither of
them things a box under a television has.

**The screen is the trust boundary, and it is the right one for this machine.** Reading a code off a
television means being in the room, which is exactly the judgment `Network reach` already makes about
a home LAN. It is what a Chromecast, a smart television and a printer all do, for the same reason.

**The panel draws a PIN or nothing, and never the words "password required".** It is the *singer's*
panel, sitting beside the QR code a phone is meant to scan, and the remote that phone opens asks for
no password: everything the door guards is under `/api/v1/admin/`, which is the owner's tools. A line
demanding a password there answers a question nobody in the room asked, and answers it as *you cannot
use this*. A PIN is the opposite case and belongs on the screen — it is the one fact about the machine
that exists nowhere its owner can reach, which is the whole of why this decision puts it on a
television.

**`factory_pin` is the whole of what the display knows about passwords.** The bit and the plain PIN
move together — an owner setting their own clears both in one write — so a bool beside the PIN in
`km_display::ConnectInfo` would be the same fact twice, and the only branch it could buy is *on a
factory password, cannot say which*, whose honest output is nothing. `GET /discover` reports the bit,
and `/admin/` and `km-admin` nag on it; the television takes the PIN alone.

**Not a constant like `2345`.** A published default would be identical on every machine in the world
and printed in this repository — a machine that looked closed and was open to anybody who had read
the README, which is worse than the honest open state it replaced. Generated per machine, it is
guessable only by somebody who can see the screen.

**Six digits, never starting with zero.** Four would match the floor and be easier to read across a
room; six is two orders of magnitude of headroom for a code that is only ever guessed through a
rate-limited login, and the difference costs nobody anything. The leading digit is not cosmetic:
`012345` is mangled the moment anything treats a six-digit code as a number, and it passes through
several forms on its way in.

**The PIN is in plain text in `settings.json`, deliberately and bounded.** A hash cannot be drawn on a
screen. The data directory is `0700`, and anybody who can read that file can read the hash beside it
and owns the box either way. `admin_factory_pin` being `Some` *is* the definition of "still on the
factory password": an owner setting one of their own clears it in the same write, so a machine can
never advertise a code that no longer works.

**`/discover` reports the fact and never the PIN.** `factory_password: true` is one bit, and it is
what lets an owner's own tools nag them — `km-admin` says so on its machine panel, `/admin/` says so
on every tab and links to the tab that changes it. The PIN itself has no business on the network; the
screen is where it lives.

**It is deliberately absent from the mDNS TXT record, and the asymmetry is the point.** Over HTTP
the flag answers a question a client deliberately asked. In a TXT record it would be shouted at the
whole segment, unasked, announcing *this machine is unclaimed*. Same bit, very different reach.
There is no `auth` record either: with a password on every machine it would be a constant.

**`discover` answers live rather than from a snapshot.** A `ConnectInfo` is built when the address
is resolved and refreshed when the address changes; a password can be set while the machine runs,
and setting one does not move an address. A snapshot would let an owner change the password from
their phone and leave every client believing otherwise for the rest of that process's life — the
routes behaving correctly the whole time, which is the worse half, because a client that trusts the
announcement stops asking.

**Clearing a password is resetting it.** `--reset-password`, the Machine tab's button and
`{"password": null}` all generate a *fresh* PIN rather than taking the door off, and the API hands
the new one back because a caller resetting remotely cannot go and read the television. It stays
`null` rather than an empty string for the reason it always was: a form submitted by accident with
nothing typed must not be the destructive act.

**Four characters is the floor.** The PIN this machine gives itself is six digits, and a floor above
what the product ships would be a rule it breaks itself. There is no complexity rule and there is
not going to be one: this guards a karaoke machine on a home LAN, and a machine that lectured
somebody about punctuation would mostly stop them setting a password at all — which is the state
that is actually unsafe.

## `/admin/` is the owner's page, and it is a third surface on purpose

**A page at `/admin/` with four tabs — Songs, Pictures, Sound, This machine — served by the machine
itself and behind the machine's own admin password.** Songs otherwise arrive by routes that each
assume a command line, a path already known or a file already in hand, and the name reaches the
network with nothing able to set it.

**This is the third page and the boundaries between the three are the decision.** `/` is the singer's
remote and stays only that. `/dev/` stays exactly as it is: one hand-written file of vanilla
JavaScript driving raw routes, useful because it is unpolished. This is an end-user surface with an
end-user's vocabulary — it says *pictures*, not *wallpapers*, and never shows a route or a status code.

**It is not a fourth tab on the singer's remote**, which would give a guest's phone controls that
delete somebody's songs.

**A crate under `crates/machine/` rather than `crates/remote/`.** It is not a remote: it configures
the box rather than driving a performance, and it is the machine's own surface even though a desktop
tool serves the same markup —
[`One page set, two hosts`](distribution.md#two-admin-surfaces-one-vocabulary) is what makes a second
host possible without making this a remote. The folder is the other half: `crates/remote/` groups the
singer's remote's five hosts because there are five, and one crate does not earn a sixth top-level
directory.

**The handlers call the machine through a trait per tab**, and the in-process implementation calls
`km_api::ops::*` and the trait objects directly — never HTTP against the machine's own API from inside
its own process. The exception proves it: the guard *does* go
through `ApiState::authorize` with an `Authorization` header rebuilt from the browser's cookie,
because one authorization path is the point. And the three upload forms go through
`km_api::uploads::receive`, the same function the JSON routes call, so the size caps, the extension
lists and the name sanitizing are stated once.

**The guard denies by default.** Every page here is an admin action, so the question is one — *is
this caller holding a valid token* — and `is_open` is an allow-list of two: the login page, which is
where a refusal sends somebody, and the stylesheet, which touches nothing. Everything else demands
the password, **including a page nobody remembered to think about**, which is the safe direction to
fail.

`tests/pages.rs` drives the *nested* router rather than unit-testing anything: whether axum reports
the full path or the inner one from `MatchedPath` under `nest` is not settled by reading its source,
both spellings are handled, and denying by default makes a wrong guess — including a third answer a
future version might invent — fail closed.

**A tab is gated on what it exists to change, not on what it shows.** Gating on a *read* would leave
the whole setup page open on a machine whose owner had set a password. Nothing is disclosed that the
API does not disclose anyway, but a page of controls that would each be refused is a bad answer to
*whose page is this?*, and it reads as broken rather than as protected.

**`/admin/` redirects to `/admin`.** `nest("/admin", …)` matches `/admin` and answers **404** to
`/admin/` — and the trailing slash is what a person types and what every link ending in a directory
produces.

**The page reloads rather than holding a stream open.** No htmx, no script at all. Slower than the
singer's remote and right here: these are things somebody does once and wants to see the result of,
not things they do forty times an evening — and it works with scripting off, on a phone browser nobody
chose. **So where this page has to ask a question, it asks with a page.**

**That is now a property of the shared markup rather than of one program**, and it is stronger for
it. `km-admin` needs htmx for its long jobs — a bank download reporting progress — and those live in
*its* fragments, which it merges over this router. The templates in `km-admin-pages` carry no `hx-`
attribute, and a test says so, because "this page has no script" is the assumption a dozen controls
here are built on: the confirmations are pages, the *show every spelling* switch is a link, and the
settings strip is `:checked ~` sibling selectors.

**A host says which scripts its own pages load, and the machine says none.** `Admin::scripts` is
empty here, so nothing this page serves carries a `<script>` at all — asserted of seven **rendered**
pages and not of the template files. **A file scan cannot see a tag that is missing**, and a host
that stops linking htmx breaks nothing a scan reads: its pages render, its tests pass, and its
progress bar stops moving. So there are two tests — the scan, for an `hx-` attribute a template might
gain, and a rendered-page test for the script a page must not have and, on the other host, must.

The tags are emitted by `shell.html`, the page a host renders its own body into, rather than by the
layout every page shares. So the tabs this crate draws are script-free on *whichever* host serves
them, which is stronger than this decision asks and costs nothing.

**No setting turns it off.** The singer's remote has `api.serve_remote`; this has nothing, because a
machine's own configuration surface is not something an owner should be able to switch off and then
need. The password is the control that means something.

**The banner nags rather than warns, and it links to the Machine tab.** What it says is that the
machine is still on the PIN it made for itself. It is on **every** tab: mentioning it only on the one
somebody may never open is hiding the most important thing about the page they are using.

**Three controls live on the Machine tab**: changing the password, ending every
session at once, and the debugging switch. The middle one is there because *sign out everywhere* and
*change the password* are different acts — an owner whose phone went missing should not have to pick
a new password and then tell the house what it is.

### The singer's remote links to it, at the foot of the Setup tab

**Nothing pointed at `/admin/` from anywhere, and an owner had to know to type the path.** The QR code
the television draws goes to `/`; the connect panel deliberately says nothing about passwords; the
singer's remote mentioned the page only in guard prose. A page reachable only by people who already
know it is there is a page found by whoever read the README, which is not the same set as whoever owns
a machine.

**So the online remote's Setup tab carries one link**, behind `Capabilities::owner_page` — on in that
mode and off in the offline app, which is `song_book`'s arrangement for `song_book`'s reason: the
machine serves that page and `km-remote-pages` does not, so the link is same-origin here and would
point at nothing there. The offline app is also the one that spends half its life talking to a box
that is switched off, which is a bad thing to put a door onto; where reaching that machine matters,
[`The machine card links out to the machine's own remote`](remotes.md#the-machine-card-links-out-to-the-machines-own-remote)
already does it.

**This does not reopen `It is not a fourth tab on the singer's remote`.** That refusal is about
*controls* — a guest's phone must not hold a button that deletes somebody's songs — and this is a
link. What a tap reaches is the login page, and every route behind it demands the token whether it is
linked from here or not: **being unreachable was never the protection, and treating it as one is how
the page came to be unfindable by the person it is for.** The singer's remote stays the singer's
because nothing on it *does* anything an owner does.

**Last on the tab, and that is the part worth arguing.** Setup's groups run machine, then this
device's preferences, then the rarest thing — and for most people holding this phone the owner's page
is the rarest thing on it. Leading with it would be the most prominent placement on the tab for the
one row fewest viewers want, on a page whose whole shape says
[`Setup is a fourth tab, and it is the narrow one`](remotes.md#setup-is-a-fourth-tab-and-it-is-the-narrow-one).

**The sub-line says the password is wanted.** A link that can only ever end in a prompt should say so
before the tap rather than after it, and the words are the machine's own — *Set this machine up*
against `admin-title`'s *Setting up the karaoke machine* — on the tie-break
[`Two admin surfaces, one vocabulary`](distribution.md#two-admin-surfaces-one-vocabulary) gives.

## The owner's page takes files, and three routes carry them

**`POST /admin/packages/upload`, `POST /admin/wallpapers` and `POST /admin/audio/soundfonts`.** New
paths rather than a content-type branch on the routes beside them: `POST /admin/packages` means
"install what is at this path on your own disk", which is what a package builder sharing the
machine's filesystem says, and one path meaning two things to two clients is a route whose permission
and whose meaning both depend on what the body turned out to be.

**Installing and uploading are both admin**, because both write to the machine's disk. The finer
distinction — the first names a file the caller must already be able to put there, the second *puts*
one there — turned on `km-package-builder` having nowhere to keep a token, and it holds a password
now. The cost is paid by a curator who does not know the machine's password, and it is a password
box rather than a wall.

**No `accept_uploads`-style setting beside these three.** Debugging mode gates the two `debug/play-*`
routes, which are public when they exist; these are admin whenever they exist, so on any machine they
are already behind the password. A second switch would be one more thing meaning almost the same
thing.

### A file that is too big is a 413 that names the limit

Every one of these routes reads its body through axum's `Multipart`, so a `DefaultBodyLimit` trip
arrives as a `MultipartError` — and formatting one with `Display` produces the fixed string *"Error
parsing `multipart/form-data` request"* whatever actually went wrong. An 85 MB package comes back with
exactly that, for a file that is perfectly well formed and simply too big.

**`status()` and `body_text()` are the accessors that tell the cases apart**, and one function uses
them. A length trip gets a **413** carrying the code `too_large` and this machine's own sentence —
*"that is larger than this machine accepts — the limit is 64 MB"* — where axum's own words are
*"Request payload is too large"* and name no number. Everything else keeps its 400 and gains multer's
real message in place of the fixed one.

**The limit is quoted from one function**, `uploads::limit_in_words`. `km-admin` prints "Up to 64 MB."
above its file chooser from it and the machine refuses with it, so a page and a refusal cannot come to
describe one number two different ways. The ceiling is rounded **up** — 2 GiB less a byte is "2 GB" —
because a limit quoted as a smaller number than the thing it refuses is worse than no number at all.

### What a chooser offers is quoted from one function too, and the picture chooser offers a camera roll

**`uploads::accept_for` composes the `accept` attribute for all three kinds, on both admin
surfaces.** The same argument as the limit one line above: the list a person picks a file from and the
list their file is then measured against are one list, or they are two lists that will eventually
disagree.

**They already were two.** `km-admin` built its three from `uploads::extensions_for`; the owner's page
typed the same three into its templates by hand. Nothing was wrong — the two agreed exactly — and
that is precisely the state a copy is in for as long as nobody changes the table. A fourth extension
would have been added in one place and refused in the other, after somebody had chosen the file.

**A picture also names a media type, and `image/*` leads its list.** A phone keeps photographs in a
camera roll rather than in a file system, and an extension-only `accept` reaches one unevenly: iOS
maps extensions to UTTypes and offers the photo library either way, while Android's chooser decides
which view to open from the media type and offers a file browser without one. **The owner's page is
the surface a phone actually reaches** — it is served by the machine, on the LAN, to every phone in
the house — so this is the difference between *Take Photo* being offered and somebody hunting for a
JPEG in Files. The order is load-bearing rather than tidy: a chooser reads the leading entry to decide
what to open.

**The extensions stay beside the wildcard rather than being replaced by it**, because `.zip` is not an
image and a wallpaper pack arrives as one. `accept` is a union, so naming both offers both.

**A package and a bank get no media type.** Neither has one a picker knows — `.kmpkg` is this
project's own, and `.sf2` is `audio/x-soundfont` at best, which nothing maps to anything — so a
wildcard would either match nothing or, widened to `application/*`, offer every file on the device.

## A page asks before it deletes a file; the API does not

**The four controls that destroy a file ask first — removing a package, a SoundFont bank and a
picture, and deleting a package the machine refused — and they ask on the pages, never on the
route.** `DELETE /api/v1/packages/{id}`, `DELETE /api/v1/audio/soundfonts/{id}` and
`DELETE /api/v1/wallpapers/{id}` answer plainly: a 200 is a 200. A JSON route that asked a caller to
confirm would be a route lying about its own contract, and every script would have to learn a second
step. The page is where a person is, and a person is what a confirmation is for. The fourth has no
route at all, which is argued in
[`The Problems tab`](#the-problems-tab-and-deleting-a-package-that-never-installed) and changes
nothing here: it is a page, so it asks.

**What makes this owed rather than optional is that an uninstall deletes the `.kmpkg`**, which may be
tens of gigabytes and may be the only copy. The bank route has the same shape and deletes 301 MiB on
Android, where no file manager can reach the folder.

**Scoped by a test rather than by a count.** Removing a picture joined by meeting that test rather
than by being similar: it deletes a file, and on the appliance that file cannot be put back by any
means the box has. Deleting a refused package joins on exactly the same terms, and more sharply — it
is a `.kmpkg` that may be tens of gigabytes and may be the only copy, and being unreadable to *this*
build does not make it unreadable to the machine that produced it. Nothing else on `/admin/` asks
twice — moving a package's bank, showing the next picture, setting the name. The interesting one to
leave alone is **resetting the password**, which is destructive and is not a file: it carries the
guard its own risk earns, an explicit `clear=yes` field, so a form submitted by accident with an
empty box cannot throw away a password somebody chose. **Proportionate guards, not uniform ones**: a
confirmation where a file dies, an explicit intent field where a setting does, color alone
everywhere else — because a page that asks twice teaches people to click twice.

**Each surface asks in the way its own decision commits it to.** `/admin/` gets a **server-rendered
confirmation page** at the same address the `POST` goes to, `GET` asking and `POST` answering, because
that page carries no script at all — so `hx-confirm` and `confirm()` are both unavailable, and a
confirmation is the one thing on that surface that must not need JavaScript. `/dev/` gets a one-line
`confirm()`, native to one hand-written file of vanilla JavaScript.

**The confirmation is a courtesy, not the enforcement.** The `POST` refuses on its own. A token or a
`confirm=1` field would invent a second enforcement point the API does not have and `/dev/` would
not send — and there is nothing here that can change under the confirmation, unlike the package
builder's bulk edits, where a *filter* can.

**Both pages name the file's size**, so the guard can tell 34 KiB from 20 GB. The size is read at the
moment somebody is asked, never stored and never on the list: a stored number goes stale, and a
`metadata` call per row would put filesystem touches on a read path that has none.

### A control that can only be refused is left out, not grayed

**`PackageDto::removable` and `SoundFontBankDto::removable`, and a page spends them by omitting the
control.** Both routes refuse permanently in two cases — a file `debug.` names, and a file outside
every folder the machine owns — and neither becomes allowed by waiting. A control that is always
refused teaches somebody to ignore the row it is in. `AudioOutputs::changeable` is the precedent for
**having** the flag and not for how it is spent: a device that cannot be changed now can be changed
when the song ends.

**One predicate, asked twice.** The machine answers "may this go?" with the *sentence* its refusal
would carry, and the route and the page call the same function — so a page that leaves a button out
leaves it out for exactly the reason the route would have given. A bare `bool` would leave the page
inventing wording for a rule it does not own.

**The sentence stays in the machine and the boolean goes on the wire.** Every reason names a full
path, and the directory layout of the machine under the television is nobody's business but the
owner's — the same rule that keeps an archive's path out of `PackageDto`. `/admin/` runs inside the
machine and prints the sentence; a remote gets the flag.

### `DELETE /packages/{id}` answers three failures three ways

Mapping every error with `|_| not_found(…)` means a package the machine refuses to delete and a disk
that would not release the file both come back as `404 package '<id>'` — the one status that says *it
is not here*, false in both cases, and it throws away the machine's own account of why.

Not installed is a **404**, refused is a **400 with the sentence**, and a file that could not be
removed is a **500** saying nothing was uninstalled. A client treating 404 as "already gone, fine" now
sees the difference, which is the point.

## The Problems tab, and deleting a package that never installed

**A fifth tab on `/admin/` gathering everything the machine found and could not use, and the one
control on it deletes a refused package's file.** `A clash warns rather than only logging` made a
refusal audible on three surfaces and none of them can act on it: the `.kmpkg` sits in the folder,
is refused again at every scan, and goes away only by reaching the box's filesystem — which is
precisely what an appliance under a television does not have. A refusal audible three times over is
one somebody has to be able to act on.

**`uninstall` structurally cannot reach these**, which is why this is a new operation rather than a
new caller of an old one. It looks a package's path up from its catalog rows, and a package that
would not open has none — so `DELETE /packages/{id}` answers 404 for exactly the files this is about.

**The tab is always in the bar, never conditional.** Three reasons in ascending order of force: a tab
that comes and goes is a navigation that changes shape under somebody; a report that disappears when
it is clean is how faults get missed; and deleting the *last* problem would make the tab vanish while
you are standing on it, leaving the redirect afterwards pointing at a tab the bar no longer has. A
count badge is what earns it its place on a healthy machine, and it is carried on the **chrome**, so
it shows on every tab — a badge is only useful to somebody who opened a different page.

**Delete, and none of the three obvious neighbors.** A *rescan* button would imply the machine was
waiting to be asked, when it already retries every file at every start and through
`POST /packages/rescan`. *Replace it in place* means writing into a file this machine has already
refused, under a name chosen by whoever made it. *Download it back* publishes an unreadable archive
over the LAN. What is left is the one thing a browser can usefully do to a bad file, and the
confirmation says the better answer out loud where there is one: rebuild the package and send it.

**The row names the folder, where every other surface names only the file.** That reverses the rule
`A clash warns rather than only logging` sets for the banner and the idle screen, and it has to: the
same package in two of the scanned folders produces two rows with identical text and two different
Delete controls, which is worse than no page. It is publishable here for the reason
`A control that can only be refused is left out, not grayed` already gives — `/admin/` runs inside
the machine and prints paths in its refusals; a remote gets a flag.

**A refused package is addressed by a derived id: the file's name, then a fingerprint of the whole
path.** Not a field on `PackageProblem` — the path is also the key a problem is remembered under, and
a stored id would be free to disagree with it. Not the name alone, for the two-folders case above.
It does not round-trip: a delete matches it against a freshly read list, never joins it onto a
folder, so an id a browser invents can only ever miss. Two files answering to one id delete
**neither** — a collision needs 32 bits to land and will very likely never happen, which is exactly
why the branch is written rather than left as a `find` that would silently take the first.

**Deleting the file releases the bank it had reserved**, and this is the second exception to
*a package that merely stopped being found keeps its reservation*. `install` calls `ensure_bank`
before indexing, deliberately, so the answer survives an install that then fails — which means a
package that opened and failed afterwards holds a thousand numbers it has no songs in, and nothing
else would ever give them back. The argument is `uninstall`'s own, word for word: deleting the file
is the owner saying it is not coming back. The rule is intact, because it is about a file that is
still somewhere and this is about one the machine has just destroyed.

**The tab reports more than packages, and every row now carries the control that fixes it.** The
sound fault, the audio device having fallen back to a second choice, and the picture fault are each a
sentence the machine already composes; two of the three appeared on no tab at all. A download in
progress is deliberately **not** here: this tab is for standing faults the machine is still holding,
and without that line it would accrete every transient toast.

### Every fault on the Problems tab draws its own control

**A page that gathers faults it cannot fix is the one failure mode this arrangement has that per-tab
notices do not.** So each row carries the control that fixes it: a bank picker, the output picker, a
file chooser for an empty rotation. A link to the owning tab is not enough on its own — the device
row's link went to Sound, which lists banks and has nothing to do with outputs, on the one fault that
means an appliance is playing to nobody after its ALSA card order moved between boots.

Three properties are load-bearing:

- **They post to the owning tab's own routes.** A fix applied here and the same fix applied on Sound
  are one act, and a second route would be a second thing to keep in step — which is the mistake
  `POST /admin/packages` and `POST /admin/packages/upload` were split to avoid making in reverse.
  `POST /admin/sound/{id}/use` is `POST /admin/sound/use` with the id in the body to allow it: a
  `<select>` cannot feed a path segment on a page with no script.
- **A `back` field brings the redirect home**, whitelisted to a fixed set of tab names, because the
  value comes off a form and reaches a `Location` header.
- **The link stays beside the control.** The owning tab knows things this one does not — sizes, where
  the pictures come from, every spelling of an output — so it earns its place beside the fix rather
  than in place of it.

**The refused-package rows are unchanged, including the ones that offer nothing.** A file under
`debug.packages`, or outside every folder the machine owns, still shows the machine's refusal in
place of Delete: see
[`A control that can only be refused is left out, not grayed`](#a-control-that-can-only-be-refused-is-left-out-not-grayed).
A machine deleting a file it does not own is a different decision from this one and has not been
made. **Nor is there a rescan button or a delete-everything button**, both of which were considered
in this change and declined — the rescan argument below still holds, and a single confirmation
standing in front of an unbounded number of deleted files is not the guard that rule asks for.

**The wording of the sound fault moved into `SoundFontStatus::complaint`** rather than being restated
here. The distinction it encodes is easy to get wrong in a way that matters — `problem` means *there
is no bank* and reads as a broken machine, `fallback` is a caveat about one working perfectly well —
and a second copy would have had the television and the browser describing one machine two ways.

**No API route, and this is the only control on `/admin/` without one.** `packages.uninstall` is the
permission, because the act is *delete a `.kmpkg` this machine found* and a new id would leave an
owner who had locked that one still holding a file-deleting control under a name they had never
heard of. What a route would buy is a script being able to do what a person can — and no client
wants it: the singer's remote must not have it, `km-admin` sends files and never deletes, and a
developer at `/dev/` has a shell and can delete the file, which is the whole condition this feature
exists to relieve. **The cost is real and is the price**: an owner who cannot reach `/admin/` has no
route to this, and their remedy is the one that has always existed rather than one this removes.
Adding the route later is cheap — the operation is on the `Catalog` trait either way — and a
published route is a contract that would then constrain the id's shape by compatibility rather than
by what the page needs.

## Changing the admin password is something a phone can do

**`POST /api/v1/admin/password`.** A command line is something the appliance has no keyboard for, and
the password on a fresh machine is a PIN somebody read off a television — so the act of replacing it
has to be reachable from the thing they are holding.

**Under `/api/v1/admin/`, so changing the password needs the current one.** The route that grants
permanent control of the machine is not one to be reachable without it.

**Every outstanding session ends on a change**, including the browser's own, and the page says so
rather than leaving it as a surprise. It is a property of the construction rather than a step somebody
has to write: a token is an HMAC keyed on the stored hash, so a different hash cannot produce the same
MAC.

**`null` resets rather than clears**, and the reply carries the new PIN. There is no state with no
password to clear it *to*; what an owner means by that button is "I have forgotten mine", so the
machine invents a new one, puts it on its own screen, and hands it back here because a caller
resetting remotely cannot go and read the television. `null` rather than an empty string, because a
form submitted by accident with nothing typed must not be the destructive act.

**Four characters is the floor and there is no complexity rule** — see
[`A machine gives itself a password, and shows it on the television`](#a-machine-gives-itself-a-password-and-shows-it-on-the-television)
for why four and not eight.

### The Machine tab changes a password, and there is no first one to set

**A Change box and a Reset button, always.** Reaching this page takes the password, so there is no
state in which a Set button on a page anybody could open would grant permanent control of the machine
to whoever pressed it first.

**Reset, not Remove.** The destructive act is going back to a freshly generated PIN, which the
television then shows. It carries a `clear=yes` field: it is destructive, and a form submitted by
accident with an empty box must not do it.

## Turning demo mode on is an owner's act; knowing it is on is not

**`PUT /api/v1/admin/demo` is behind the password, and `GET /api/v1/demo` is not.** Turning demo mode
on makes the box **start playing music by itself, indefinitely, in somebody's house** — and with
`persist` it goes on doing so after the power is cut and restored. That is not a knob a guest with the
address should be able to turn, and not the kind of thing anybody should have to work out how to undo.

Reading it stays public for the same reason the debug play routes are public while the switch that
mounts them is not: **using the answer and changing the state are different acts.** A remote has to
know a demo song is on the deck before it can tell a singer that this one will not run out on its own
— the next starts the instant it ends — so what gets them a turn is queueing, which takes the deck
off a demo at once. The answer says nothing a person in the room cannot already hear.

**The switch is for the run unless it is asked to persist**, which is the shape of the request body
rather than two routes. `{"enabled": true}` changes the running machine and nothing else; adding
`"persist": true` also writes `demo.enabled` into settings. The un-persisted form is the default
because it is the smaller surprise: a party is a run, and somebody who switched the machine on for an
evening should not have to remember to switch it back — where a switch that turned out to be permanent
is discovered a week later.

A persisted change is written to disk immediately rather than at shutdown: somebody is standing in
front of the machine saying what it should do when they are not, and a power cut before the next clean
stop must not quietly undo it.

**Switching demo mode off does not stop the song that is playing.** It is a mode, not a transport
command, and `POST /api/v1/transport/stop` is the thing that means stop.

**The owner's act has an owner's control, on the owner's page.** *This machine* in `/admin/`
carries both boxes — one for tonight, one for after a restart — and `km-admin` carries the same two
under the same words. An admin-only route reachable from nowhere an owner goes is a feature nobody can
use.

**Two boxes and not one**, because the route takes two answers and a page that hid the second could
not express the state the route can: *on tonight, off tomorrow*. `persist` is unticked by default
here for the same reason it defaults to `false` in the body.

**The singer's remote cannot reach it**: that page has the one-shot *play something* button and
never the mode — see
[`The remote can ask for a demo song without turning demo mode on`](interface.md#the-remote-can-ask-for-a-demo-song-without-turning-demo-mode-on).

**The machine's own keyboard reaches it, and that is not a hole in this.** `D` at the machine turns
the mode on and off without a password — see
[`Demo mode has a key on the machine's own keyboard`](interface.md#demo-mode-has-a-key-on-the-machines-own-keyboard).
What this route gates is somebody who found the box *on the network* making a room they are not in
play music indefinitely; whoever presses the key is standing in that room, and it is what the key
cannot do that keeps the two consistent — it never persists, so the answer to "what should this
machine do when nobody is here?" is still only ever given through the password.

**The admin page's row needs no mark of its own.** Every page there is behind the password, so the
row that turns demo mode on is gated exactly as the rest of the tab is.

## The demo delay is a route of its own, and it is always written down

**`PUT /api/v1/admin/demo/delay` takes `{delay_secs}` and nothing else.** A plain number in
`settings.json` needs nothing discovered before somebody can name it, which is the test that kept it
off the API — and it is the wrong test on the machines this is for: **an appliance under a television
has an owner's page and no text editor.** A delay nobody can reach is a delay every house gets whether
it suits them or not.

**A path under the switch rather than a third field in its body, and `persist` is the reason.** `PUT
/admin/demo` is shaped around *for tonight or for good*, because a party is a run. A delay is
installation configuration, in the family of the machine's name and its locale, and is **always
written to settings**. A body where `enabled` obeyed `persist` and `delay_secs` ignored it would make
one flag mean two things. Two forms with two Save buttons is what that costs on the page.

**Admin, for the switch's reason and one of its own.** The smallest legal delay is zero, so somebody
who could set this could arrange for a house they are not in to sing the moment it goes quiet — which
is [`Turning demo mode on is an owner's act`](#turning-demo-mode-on-is-an-owners-act-knowing-it-is-on-is-not)
reached by another door. Reading stays public on `GET /api/v1/demo`: using the answer and changing the
state are different acts.

**An hour is the cap, and it bounds the route rather than the setting.** Past that nobody can tell a
demo from a machine that never performs, so a larger number is likelier a typo than an intention, and
what sends one is a box on a page reached from a phone. `settings.json` is uncapped: somebody editing
their own machine's file is being deliberate. Over the cap is a **400 rather than a clamp** — a
machine that quietly stored a number nobody chose is worse than one that said no.

**Changing the delay shifts the running deadline rather than re-arming it.** The deadline is *when
somebody last did something, plus the delay*, so a new delay measures from that same moment: quiet for
fifty seconds and told to wait sixty leaves ten to go, and told to wait thirty means the next poll
starts a song. Re-arming from now would make *shortening* the delay lengthen the wait, once, in front
of whoever had just shortened it to find out whether it worked. It is
[`the clock counts idleness`](interface.md#what-the-machine-does-when-nobody-is-singing) from the
other side.

**`min_suitability` stays off the API.** It is the one demo key a person cannot judge from the room:
a delay is a length somebody feels, where a floor of 5 against 6 is a claim about what a packager
measured, and choosing it wants the catalog in front of you rather than a phone.

## Starting one demo song is anybody's; turning demo mode on is not

**`POST /api/v1/demo/start` is public**, beside a `PUT /admin/demo` that is not, and the difference
is one song against a mode. The pair to compare it with is one screen over: advancing the picture is
public and adding one to the rotation is not.

**Three things make it a smaller act, and all three are enforced rather than argued.** It is *refused*
unless the deck and the queue are both empty, so it interrupts nobody and takes no turn away — where a
skip over a song does both. It does **not** require demo mode and does not turn it on: chaining is what
`demo.enabled` buys, so with the mode off this is exactly one song and then silence. And it writes
nothing.

**The mode is what keeps this route worth having beside `POST /transport/skip`**, which reaches the
same flag on an empty deck by
[a rule of its own](interface.md#skip-into-silence-asks-demo-mode-for-a-song). That press needs the
mode on, so on the machine this route exists for — one whose owner never turned it on — this is still
the only way to ask.

**A trigger that first needed the admin password would be useless to the room it exists for.** The
feature's whole argument is that somebody should be able to hear what the box holds without first
working out how to drive it.

**It answers `no` synchronously and answers `yes` on a promise.** The three refusals — something
loaded, something queued, no sound at all — are knowable without touching the catalog, so they come
back as a 409 `unavailable` carrying a sentence a remote can put on screen unchanged. A *fourth*
possible failure is deliberately not among them: a catalog with nothing playable in it can only be
discovered by the two full-table draws the picker makes, which is exactly the work this route exists
to keep off a request thread — and a machine with no songs says so on every other screen.

**The route sets a flag; the poll thread starts the song.** Every demo start already happens on the
one poll thread, which is what makes the check-then-start there safe with no lock spanning the load.
Starting a song from a request thread would race that check, and two starts means the second load cuts
the first song off a few milliseconds after it began. So the press sets `demo_once` and the next poll
acts on it, within fifty milliseconds, with one writer throughout.

**The flag is not the deadline moved forward, and that distinction has a cost attached.** Setting
`demo_resume_at = now` looks equivalent and is not: on a machine where the mode is on and somebody
skipped ten seconds ago, a trigger that then found nothing to play would also have canceled the
minute of silence that skip bought them. A separate one-shot leaves the ordinary clock alone. It is
spent by the attempt it causes — including an attempt that found nothing, which is what stops an empty
catalog being retried twenty times a second — and canceled by anybody who queues, skips or stops
first.

**The body describes the machine as it stands, not as it will be.** `playing` is false in the answer
and `starts_in_secs` is `None`, because the song has not started yet. What a caller wants from it is
`enabled`, which tells them whether the one song they asked for will be followed by another.

## A refusal travels as a code, and whoever shows it writes the sentence

**The machine composes in English because it has no idea who is reading**, so a remote that renders
the `message` beside a 409 puts an English sentence inside a Portuguese page.

A refusal a *singer* can provoke therefore carries a stable code in the `error` field, and the surface
showing it looks the sentence up in its own catalog: `no_key_video`, `nothing_playing`, `no_melody_channel`, `no_sound`.
**A remote renders the code and never the `message`**, which stays for the log and for a JSON client
with no catalog.

**A refusal only the *owner* can reach carries no code** — a SoundFont that will not load, a path
outside the allowed roots, an upload that could not be written. Those are diagnostics read beside a
log, and a code per case would say less than the sentence does.

**The kind of song rides in the code**, which is why there are three of each rather than one:
English writes `a video song has no key` and Portuguese writes `uma música em vídeo não tem tom`, an
article that agrees with the noun. A sentence the machine already composed cannot be taken apart
again, and reading the kind back out of it would be a parser for prose the API documents as unstable.

**An unknown code is not an error.** It renders as the generic refusal, so an offline remote talking
to a machine of another version degrades to a plain sentence rather than to a blank, a shrug, or a red
failure. The two
vocabularies are deliberately not one dependency: the machine links the pages, so the codes are
spelled on both sides and a test in the machine is what stops them drifting.

**A 400 gets no code.** It says *fix what you sent*, so it is aimed at
whatever built the request rather than at the person holding the phone — the remote clamps a key
change before it asks, so a singer never sees one.

## Power is a capability of the host, not a method on the machine

The three power routes — `GET /api/v1/admin/power`, `POST /api/v1/admin/power/off` and
`POST /api/v1/admin/power/restart` — are **mounted only on a host that can do something about its
own power**, and a machine that cannot answers a real 404 on all three.

**That follows a rule `Controller` already states rather than inventing one.** That trait is the
things every host can do — playback, the queue, the settings that belong to a performance — and it
has no defaulted methods by decision, because a default would be a lie a test double then tells
quietly. Its own note says what to do with a capability that genuinely varies: *"the honest shape is
a route that is **not mounted**, so a machine without debugging answers a real 404 rather than a 409
about a method that quietly did nothing."* Powering a box off varies exactly that way — an Android
television cannot, a desktop must not, and only a supervised appliance both can and should — so it
is a separate seam, and `km-api` implements none of it. The binary crate supplies the
implementation, the same division the catalog and the audio device already have.

**They are admin routes, where the debug pair is public, and the asymmetry is not an
inconsistency.** Debugging is a *mode* somebody switches on, so its routes are absent or public;
power is a *capability* the host has or has not, so its routes are absent or the owner's. Both obey
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission), and the one thing that must
never happen — a power route outside `/api/v1/admin/`, which would let anybody on the LAN switch the
television off — has an assertion of its own beside the mirror-image one for the debug pair.

**`GET /discover` says nothing about this.** It is public, so a field there would tell the whole
network that this machine can be powered off remotely, which nothing needs to know before it has a
password.

**Shutting down asks the operating system and stops nothing itself.** The supervisor notices the box
going down and stops the unit the way it stops it for any other reason, which runs the one shutdown
path that already exists and is already tested — byte for byte what the physical power button does.
Setting the machine's own shutdown flag *as well* would race that: the process would begin persisting
settings and dropping the audio device while systemd was separately stopping it, and the two orderings
would interleave differently every time.

**Restarting is an exit, not a request to the supervisor**, and that is forced rather than chosen.
Asking systemd to restart a unit is `org.freedesktop.systemd1.manage-units`, which an unprivileged
account does not get; exiting needs no privilege whatsoever and `Restart=always` does the rest. So
the route sets the one existing stop flag and the ordinary shutdown block runs. There is still one
exit and not three.

**The availability gate is `INVOCATION_ID`, not "would the operating system allow it".** Those come
apart exactly where it matters: a developer running the binary from a terminal on a Linux desktop is
inside their own active logind session, so logind *would* switch their desktop off. Offering that is
the same category error as `systemctl enable` in a `postinst`. systemd sets `INVOCATION_ID` in every
unit's environment and a `cargo run` never has one, so it answers *"I am supervised"* — which is
simultaneously the question a restart needs answered, since exiting only starts the process again if
something is watching. One capability covering both actions rather than two flags, because the two
conditions coincide.

**Deliberately not probed: whether the session is Active and whether polkit will allow it.** Both are
true at one moment and false at the next — somebody switches virtual terminal — and both come back
from `systemctl` as a readable sentence. Reporting them as *unavailability* would hide a control
because of a condition that no longer holds, where
[`A control that can only be refused is left out, not grayed`](#a-control-that-can-only-be-refused-is-left-out-not-grayed)
draws the line at *permanent*.

**Both answer `202`, and mean it.** The box has not powered off when the body is written and the
caller will get no later word from a machine going dark, which is the literal case the code exists
for. A refusal therefore cannot be a status: it arrives after the answer has gone and is a journal
line, carrying the operating system's own sentence. *"Interactive authentication required."* is
something somebody can search for; "power off failed" is not.

## The machine's own log is a route, and it is the owner's

`GET /api/v1/admin/logs` answers the last few hundred lines the machine said; `GET
/api/v1/admin/logs/stream` sends those and then the ones that follow. The machine keeps them in a
bounded ring in memory, filled by a third `tracing` layer beside the console and the file.

**What this reaches is the machine whose log is hardest to get.** A box under a television has no
console and nobody logged into it; a run started by double-clicking its icon on Windows has a null
standard output handle and discards every line; a file means finding the folder, over SSH or `adb`,
after the moment has passed. This is the same stream at a third destination, and the destination is
one a person already has open.

**Admin routes, like power and unlike the debug pair.** A log line names the file the machine
opened, the folder it scanned, the address it resolved, the audio device it found and the name its
owner gave it. Reading that is the owner's business, and
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission) leaves no third state to put
it in.

**They carry paths on purpose, where the rest of the surface strips them.** `PackageDto` keeps an
operator's paths off the wire and a fault's reason names a file rather than its location, and both
are right for what they serve: a row on a singer's remote spent on `C:\Users\…\` is a row wasted. A
log line without its path says nothing at all. The audience is what differs, which is exactly why
these two are the routes behind the password.

**Which makes one standing constraint.** Anything ever written to this log becomes readable by
whoever can reach a machine with the development console on. A line that would say a password out
loud is a fault where it is written, not here.

**On the dev mirror, where the power routes are not.** That exception is about a change nobody can
undo from a page, and reading a log is not a change at all — the mirror already carries a route that
plays any path on the machine's disk to whoever asks. A tail is well inside a bargain that includes
that. The two conditional admin surfaces therefore part company at exactly one point, and each has
an assertion naming it.

**Absent rather than empty on a machine that keeps none.** The routes are mounted where a tap was
installed and answer a real 404 where none was, which is the shape `Controller`'s own note
prescribes and the one the power routes already take.

**The tap is unconditional, where the file beside it is asked for by name.** That is not a
disagreement with [`A log file for the runs nobody is
watching`](distribution.md#a-log-file-for-the-runs-nobody-is-watching) but the same argument reaching
a case where the price is different. A file costs a directory that fills up, so somebody decides; a
bounded ring costs a fixed few hundred kilobytes, so nobody has to. And the run that most needs a log
is always the one nobody armed — by the time a person wants the last hundred lines it is too late to
begin keeping them. Holding them publishes nothing, the routes being what governs reading them.

**The verbosity ladder still decides what goes in it**, which is the half that decision settles
outright: a level says how much detail, and where the detail goes is a different question. `-v` and
`RUST_LOG` reach the ring exactly as they reach stdout. The directive in force travels with the tail,
because a machine started without `-v` keeps nothing below `info` and a pane with no `debug` lines in
it is otherwise indistinguishable from a machine with nothing to say.

**A record is structured, not a formatted line.** The level, the target, the message and the fields
arrive apart, so a reader can colour by one and filter by another without parsing text. The time is
milliseconds since the epoch and nothing formats a clock on the machine's side — whoever draws it has
a locale and this has no date library to get one.

**A stream carries frames, and falling behind is a frame of its own.** The alternative was a
synthetic record saying so, which forges a line the machine never emitted: indistinguishable from a
real one once somebody pastes the pane into a report, and filtered away by a reader hiding everything
below `warn`. The event stream draws the same distinction for the same reason.

**A reader is given a tail and a subscription that overlap**, and drops the overlap by the sequence
number every record carries. The ordering that cannot duplicate is the one that loses records taken
while a reader is arriving, which is exactly when a machine is busy enough to be worth watching. A
duplicate a reader can see and discard beats a gap nobody can.

**Nothing serving this may say anything.** A line emitted while feeding a reader is taken by the tap
and sent to that same reader, and on the arm that reports falling behind it feeds the reader least
able to keep up.
