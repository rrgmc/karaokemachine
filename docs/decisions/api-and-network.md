# The API and the network

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## The URL prefix is the permission

**One shared admin password, and everything it guards lives under `/api/v1/admin/`.** A path under
that prefix demands a valid token; a path outside it never does. There is nothing to configure and
nothing an owner can get wrong.

**There is no `public`/`admin` map in `settings.json`.** A table of route ids is free to drift out of
step with the router it describes. The freedom that forty-six entries bought was closing
`packages.install` on one machine, and nobody ever used it.

**A reader can see the permission in the URL, and a test cannot be written that does not check it.**
`routes::needs_admin_token` is four lines over the request path, and `ApiState::authorize` is its only
caller. A route added under `/api/v1/admin/` is gated the day it is written, not when somebody
remembers to add a row. `the_admin_prefix_is_exactly_what_needs_a_token` sweeps the whole surface and
asserts both directions of that.

**The prefix test carries its trailing slash.** `"/api/v1/admin"` alone matches `/api/v1/adminfoo`.
Far worse, in the other direction it would silently protect a future `/api/v1/administration` under a
rule nobody had written down. A unit test pins all three spellings.

**The cost is a resource split across two prefixes.** `GET /api/v1/demo` and `PUT /api/v1/admin/demo`
are the same thing filed in two places, and so are the audio, package and wallpaper pairs. A REST
purist would put them together and use a table to say which verb is privileged. This rule does without
that table.

**One exception, and it is the way in.** `POST /api/v1/admin/login` is under the prefix and needs no
token, because a login behind a token would make the password unusable. The rate limiter defends it
instead.

**There is no `settings.transpose` or `settings.melody`, and there cannot be.** A key change is the
same permission as any other setting: a performance knob a singer reaches for. So it sits outside
`/admin/` with search and queueing, permanently.

## Network reach

**The machine binds `0.0.0.0:8177` out of the box.** A default reaches only an install with no
settings file; one that has a file keeps what it says.

A karaoke machine whose remote nobody can connect to is the broken case, not the safe one. mDNS, the
QR code and the connect panel all exist to serve a phone, and every one of them is inert behind a
loopback default.

This default comes with a password the machine gives itself and a fixed set of admin routes. Together
they are a judgment about a **home LAN**. Anybody in the room can queue a song, and that is the design.
Everything that reconfigures the machine is behind a code on its own screen.

**It stops being right the moment port 8177 is reachable from outside one**, and then what changes is
the password, not a list of routes. A generated PIN is fine for a room and is not a secret from the
internet. So an owner forwarding the port should set one of their own first, and the banner on
`/admin/` says so until they do. `bind: "127.0.0.1:8177"` shuts it back in entirely.

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
notice and restart, and nobody was watching when the socket went. The machine binds again to the
address it already holds, not to the one it was asked for. By then that address is on the television,
in a QR code and in whatever a phone remembered. A machine that came back on a different port would be
a machine nothing can find, which is the fault it was recovering from.

**Coming back to the screen is a reason to take the port again, and the machine does not check
first.** A suspended application can get back a descriptor that neither accepts a connection nor
reports a failure. That is indistinguishable from a network nobody is using. Only the host knows it
has returned, so it says so. Proving a socket still works costs a connection to it and answers for
that instant alone, where replacing it is a bind.

**While there is no socket, the screen says so and no address is published.** The connect panel names
the reason in place of a URL, with the same sentence a port already in use puts there. A machine
claiming an address that answers nothing is worse than one admitting it has none. Every remote
believes the claim, and the person standing in front of it has nothing to go on.

## Debugging is a mode, and the machine says when it is on

**`debug.enabled` governs the whole `debug.` section of settings, and while it is off the two
`debug/play-*` routes are not mounted at all.** Off means a 404, genuinely not there, rather than a
refusal. `play_file_roots`, `packages`, `wallpapers`, `soundfonts` and `soundfont_slot` are all
ignored with it.

**One switch, not one per route.** Every route that writes to disk is behind `/api/v1/admin/`, a
password always exists, and the package builder can hold one. So a narrow switch over the upload route
alone would have nothing left to do.

**One switch over five settings rather than five that each mean something slightly different.**
`debug.` is the one place `settings.json` may name an individual file, under the standing decision
`Only debug. names a file`. So its entries are exactly the ones that point the machine at arbitrary
paths. An owner deciding about them one at a time is an owner deciding five times about the same
question.

**A populated section with the switch off is logged, not silently ignored.** One `warn` at startup
names each field it is passing over. Dropping a value without saying so is the trap. An owner whose
machine quietly stops honouring `debug.soundfonts` concludes that the soundfont switcher broke. The
real cause, a switch they have never heard of being off, does not occur to them.

**The two play routes are public when they exist, and are deliberately not admin routes.** A third
state, mounted but password-gated, would put a token-demanding path outside `/api/v1/admin/`. The URL
would then stop being the permission. On means open; off means absent; there is no middle.

**`PUT /api/v1/admin/debug` is what moves it, and `GET /api/v1/debug` reports it.** Turning it on is
an owner's act, because it opens a route that plays any file the machine can read. A curation tool
needs to know whether it is on before it offers a Play button, so that half is public. `/demo` makes
the same split one screen over.

**It takes effect at the next start, and every surface says so.** The router mounts the routes when it
is built rather than checking per request. That is what lets an unmounted route answer 404 instead of
carrying a disabled handler. The settings half takes effect at once. That asymmetry is the price of
the clearer surface.

**A debug build has it on and a shipped one does not**, through a three-state `Option`. An owner's
`true` or `false` always wins; **absent** means nobody has said and follows the build. In a checkout
the person running `cargo run` is the owner. On Android there is no command line at all, and
`settings.json` sits in app-private storage reachable only through `adb`.

The `Option` is load-bearing, because the machine saves settings by serializing the whole struct. A
plain `bool` defaulting to the build would write `true` into a device's file on its first debug run.
It would stay there when the same device took a release APK over the top.

**Three screens carry the switch, and that is not duplication.** The owner's page at `/admin/` carries
it, because it is the machine's own configuration surface. `km-admin` and `km-package-builder` carry
it, because a curator meets the closed door there and both hold the password. The dev remote carries
it too, and reaching *that* means starting the machine with `--dev-remote`. See
[`Dev remote in release builds`](remotes.md#dev-remote-in-release-builds).

**Discovery says whether a machine is in the mode, and a tool asks before it sends.** Refusing *after*
an upload does not reliably refuse at all. A server that answers mid-request and closes leaves the
sending half looking like a dropped connection. The most useful message, the one naming the setting to
change, then arrives as "the machine is not answering", if it arrives. With a four-byte fixture that
looks like a **flaky test**; with a video it is the normal outcome. `debug_enabled` on
`GET /discover` costs one cheap round trip on a call the tool already makes.

**The same reasoning applies to `packages.upload`.** An admin route puts a 401 on a multipart stream,
which is the same failure with a different cause. So `km-package-builder` checks its own token before
sending a byte, rather than discovering the refusal a gigabyte later.

**`km-admin` does the same on all three of its sends, and it is the tool that needed it most.** Its
files arrive from a browser. A send with no password behind it would have the *page* push two
gibibytes across loopback and this program stage them to disk. A machine that could have said no
before any of it moved would then refuse them.

The check is in two places on purpose. Early in the request handler, it saves the transfer. As a floor
in the client itself, it covers a call site that forgot. Two call sites do exactly that, sending a bank this program downloaded and a wallpaper pack
it built, and neither goes through the handler.

**The refusal names a tab rather than restating the wire's words.** Songs, Pictures and Sound are
exactly the three pages with no password box on them. There, *"this machine has a password and this
program has not been given it"* is true and useless. So the refusal says to type it on the Machine
tab. None of this covers a token that has **expired**, which still fails on the wire. The program
drops the stale token there, and the next page draw asks again.

**Nothing staged for an audition outlives the song that played it.** That covers more than the *run*.
An audition is never cataloged, and `play_audition` only ever resolves the folder of the upload in
hand, so **it can never be played again**. The moment it stops being the loaded song, it is a gigabyte
of scratch nobody will read.

On a television that folder is app-private storage with no other reclamation at all. So the folder goes when the song is displaced, at the two places that write
`state.loaded`. Between them they cover a song ending, Stop, Skip, the next queue entry, a demo and a
second upload.

**It is tried at once and looked at again only if something refused**, rather than deferred on a
timer. The engine does not hold the file: `VideoSong` and `CdgSong` own the reader, and dropping one
joins the decoder thread. So the drop at displacement is synchronous, and the only holder left is the
display thread's per-frame `Arc`, which lasts one drawn frame. Eight looks at 250 ms is two seconds.
After that the holder is an antivirus scan or an `adb pull`, not anything this process owns. The
warning then says so once, instead of scanning the folder every quarter second all evening.

**Two folders are protected deliberately**: the song playing cannot be deleted, and the upload still
arriving must not be. A gigabyte over Wi-Fi takes minutes, so a first song ending mid-upload would
otherwise take the folder being written into. That second folder is the only thing about an audition
the machine remembers between requests. It has to be, because no later request can tell a folder being
written into from an abandoned one. Two curators uploading at once still leave the older staging
unprotected, which is accepted: this is a single-curator route.

**The purge at startup and the sweep on the way in are backstops rather than the mechanism.**
A `SIGKILL`, a power cut or an Android low-memory kill reaches no hook. So the whole folder goes at the
next start, when nothing can be open. A clean stop purges too. That reclaims the space immediately on
Linux and Android, where unlinking an open file succeeds, and falls back to the startup purge on
Windows.

## The development console has an API that needs no password

**The whole API is mounted a second time at `/dev/api/v1/`.** Nothing under that prefix asks for a
token, including the paths filed under `/admin/`. It exists only while **two** switches are on:
`debug.enabled` and `api.serve_dev_remote`. `--dev-remote` turns on both for one run.

**The reason is where the permission sits, not how the console works.**
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission) puts every write under
`/api/v1/admin/`, and nine of `/dev/`'s calls are writes. Without the mirror, a page whose entire value
is being unpolished and immediate would need a password before the interesting half of it worked.
Routes that move out from under the console also break it, and nothing checks the paths it names. A
surface you use to find out whether the API is complete must not be the last to hear about a change
to it.

**It follows the prefix rule rather than carving an exception out of it**, and that property is the
point. `/dev/api/v1/admin/password` does not begin with `/api/v1/admin`, so `needs_admin_token`
already misses it. There is no second rule, no allow-list, and nothing for the one middleware to
special-case. The tests name `DEV_API_PREFIX` anyway and sweep the whole of `SURFACE` under it. A
later edit can close behaviour that rests on the shape of a `strip_prefix` in silence. Loosening that
test to a `contains` would shut the console with every other test still green.

**Two switches instead of a password, and the second one is the decision.** Requiring `debug.enabled`
as well keeps a passwordless copy of the API more than one tick away from a machine in a living room.
It also lines the console up behind the switch this product already treats as *this machine is being
worked on*. `/discover` reports that switch, and both admin surfaces warn about it. And the console
cannot be reached on a machine whose owner has not also accepted the debug routes, which are the same
class of thing.

**The cost is real and is the price.** With both switches on, anyone who can reach the machine can
change its password, delete its packages and end every session. They need no credential at all. Three
things bound that rather than one:

- The pair is off in every build and in `ApiConfig::default()`.
- The machine draws a marker on its own television for as long as it is true. See
  [`The machine says on its own screen when it is in developer mode`](interface.md#the-machine-says-on-its-own-screen-when-it-is-in-developer-mode).
- Every surface carrying the switch says in as many words what it opens.

**There is no confirmation page.** This is a switch for somebody who is working on the machine, and a
page that asks twice teaches people to click twice. The rule
[`A page asks before it deletes a file; the API does not`](#a-page-asks-before-it-deletes-a-file-the-api-does-not)
already scopes to files, and no file dies here.

**`GET /api/v1/dev-remote` and `PUT /api/v1/admin/dev-remote` are the switch**, the same public-read
and admin-write split `/debug` and `/demo` make. The reply carries **two** fields. `enabled` is this
switch's own position, and `served` is whether anything is actually up. One of them alone explains
nothing. An owner who ticks the box and finds `/dev/` answering 404 needs to be told that debugging is
the missing half. A route reporting only its own state cannot tell them.

**`DebugDto` carries `stored` beside `enabled`, and without it the page is wrong.** Both switches take
effect at the next start, and the running value is a snapshot taken when the router was built. A page
drawn from the snapshot alone shows *Turn debugging on* both before the press and after it. A reply
echoing the request tells a caller the surface has closed while it is still open. `enabled` is what is
running and `stored` is what the next start will do. They disagree for exactly as long as it takes to
restart, and every surface says so while they do.

**`/dev/api/{*rest}` is mounted whether or not the mirror is.** With the console off and no such
route, that path has nothing above it and falls through to the root fallback. A JSON client then gets
the HTML landing page with a 200 and tries to parse it. The console's own address box lets somebody
point it at a machine whose console is off, and then *every* call does that. It is the same trap
`/api/{*rest}` exists to close, and the fix is the same 404.

**The page keeps its address box and has no password box.** Pointed at a machine whose console is off,
it gets an honest 404 per call, which its own log pane shows plainly. It never gets a 401 it has no way
to answer.

## The ports are adjacent, and the order is the order they arrived

**8177 the machine, 8178 the package builder, 8179 the offline remote, 8180 KaraokeMachine Admin.**
Each new program takes the next number. The constant lives in the crate that serves on it, not in a
table all four read.

**Adjacency is the whole feature.** All four can run on one desk at once, and what somebody has in
front of them is a URL in a browser's address bar. `:8180` answers "which of these am I looking at?"
without a lookup, and that is worth more than any scheme that grouped them by kind.

**Sequential rather than reserved.** The alternative is picking a block and defending it. The only
thing that would buy is the freedom to insert a program between two others, which would immediately
break the property above.

Three of the four bind **loopback** unless asked otherwise. The machine is the exception, because a
karaoke machine no phone can reach is not one. `km-admin`'s `--lan` prints a warning when used, since
that program has no password and holds whatever API keys it has been given.

## A second machine on one box

**`--api-bind` moves the API for one run and is never written down.** Every other way to move it says
where the machine lives from *now on*. That is right for the appliance. It is wrong for two copies
running side by side on a development box, where the second must not quietly rewrite the first one's
home. So the flag reaches `ApiConfig` and never `Settings`.

A bare `--api-bind 8277` keeps whatever interface `api.bind` names and moves only the port. A full
`--api-bind 127.0.0.1:8277` says both, and **on Windows that spelling is the one to prefer**. A
listener on `0.0.0.0` raises a firewall prompt per program and port, and a loopback one raises none.

**Pair it with `--data-dir`, or it is half a separation.** Two machines on two ports can still share
one catalog, one packages folder and one settings file, and then they are not two machines.

**`--data-dir` moves the data and nothing else**: settings, catalog and packages, never the assets.
Assets ship with the build rather than accumulating with use. So the command line takes
`Paths::data_rooted_at`, which leaves asset discovery exactly as a run with no flag would find it.
Moving the assets too gives a second machine with no SoundFont, no font and no wallpapers. It comes up
on a sine test tone over a plain gradient.

A value that will not read is refused rather than defaulted. The default would be the port the flag
was typed to avoid. The failure would then present as the *first* machine losing its remote.

## Discovery

**mDNS/DNS-SD advert**, a public `/api/v1/discover` endpoint, and the URL plus a **QR code on
screen**.

## The advert names the address the machine chose

**A `url` TXT record carrying the machine's own preferred URL, and every browsing client prefers it to
guessing.** The advert still publishes every reachable address as an A record. The addition is that
the machine says which one it means.

**The ranking does not survive the trip.** `km_api::connect` picks the address to show by **interface
name**. That is how a Hyper-V switch, a WSL adapter or a VirtualBox host-only network ranks below the
Wi-Fi card. An A record is a bare number with no name attached. A browsing client is left with
`rank_of_address`, which knows only which private range an address is in. It cannot separate two
adapters inside one range.

**And a client does not see the whole set either.** `mdns-sd` sends only the addresses on the subnet
of the interface a packet leaves by. It resolves a service as soon as one address has landed. A
machine holding four addresses announces them one per packet. A client that reads the first
announcement and stops is not choosing badly; it is not choosing at all.

`browse` merges every announcement of a machine, which is right regardless. The TXT record makes it
unnecessary to wait for the merge to be complete.

**The record is shape-checked and not otherwise trusted**: `http://<IPv4>:<port>`, nothing else. A
hostname would hand the choice to DNS; another scheme or port would let an advertisement point a
client off the machine entirely. It deliberately does **not** have to be one of the announced
addresses. A client on one subnet legitimately never receives the A record for the address the
machine picked. Anybody who can write this TXT record can already write an A record. So the check
exists to stop that reach widening, not to close it.

The cost is one obligation on the advertiser: a change of *preferred* address is a reason to
re-register. `advert_action` compares the first entry in place and the rest as a set. So the anti-flap
property stays exactly where it was earned.

## A machine is known by its id, and its address is a cache

**Every client anchors on the machine's instance id and treats the address as something it can be
wrong about.** A URL is the one thing about a machine on a home network that does not hold still. The
router hands the lease to something else overnight. The address a device woke up holding is then a
printer or nothing at all. Meanwhile the machine itself is on, announcing itself, and unreachable to
the only client that wanted it.

**The identity is the instance id.** `new_instance_id` mints eight random bytes on first run,
`machine.instance_id` persists them, and they go out as the `id` TXT record and in `/discover`.

**Eight random bytes and not a `uuid` crate.** Sixty-four random bits minted once and never changed
*is* what a UUID would be here. This one is already published, already persisted, and three programs
already parse it.

**The record is one type in `km_api::discover::known`, and the storage is deliberately not shared.**
`Known` carries the id, the address, the name and when the machine last answered. The remote and
`km-admin` keep one as JSON in their data directories. The package builder keeps the same fields in
the *workspace* database, because that address is corpus-scoped. A second computer opening the same
corpus then talks to the same machine. **What is shared is the type and the policy, not the file.**

**The id is optional.** A record with none behaves as a bare address. A machine pinned by hand has
that until `/discover` answers and fills it in.

**A device that knows which machine is its own may connect to that one and to no other.** Until it
knows one, it takes what the network offers. That is how a remote opened for the first time finds the
machine in the room. Once something has answered and said what it is, the only address the device
moves to on its own is that machine's. A machine switched off for the evening and a remote carried to
another house are the same situation seen twice. In both, a device must not quietly go and live on
somebody else's box, with their catalog on screen and no sign of why.

**`anchored` is that question in one word, and the id is what answers it.** A record carrying none
names an address and no machine. So it anchors nothing and behaves as a bare address exactly as it
always has. `adopts` answers only what is left. On a device that knows of no machine, a remote opens
on whatever is in the room. A tool on such a device waits to be pointed at something.

**Moving to a different machine is a person's act.** The person types an address, or presses a
machine on the list a look turned up. The offline remote's *Rescan* offers rather than takes, for the
same reason `km-admin` and the package builder list rather than set.

**`choose` is one pure function and there is no second one.** A pin beats everything, because somebody
naming an address is giving an instruction about an address. Otherwise the remembered id, seen
announcing itself somewhere else, is where that machine is. The interesting split is over what is
answering at the *old* address:

- A **different** id means the address now belongs to somebody else. That is the overnight-DHCP case,
  and only an identity can see it.
- The **same** id means one machine reachable two ways, so nothing moves.
- Nothing having said yet means stay while the record is fresh, and move while it is stale.

**Staleness changes eagerness and never correctness.** `STALE_AFTER` is six hours, and the number
matters less than the shape of being wrong about it. A lease expiring on a schedule does not move an
address; a machine switched off overnight does. So six hours is longer than an evening's use and
shorter than a night.

Crossing it too eagerly costs one comparison against a registry that is already current; too
late costs an evening on an address nothing answers. **The remembered address is used immediately
either way**, because opening instantly beats opening in a second and a half.

**A machine answering at a *remembered* address does not get to become the machine.** Take a remote
holding a record for machine X, opening on the address it last saw X at, and meeting machine Y there.
It would overwrite its own record with Y before `choose` ever ran. `choose` would then look for Y, find
Y exactly where it was, and correctly conclude that nothing had moved. **The one case an identity
exists to catch would be the one case it could not catch**, and nothing in the test suite catches
it.

**The distinction is who chose the address.** `asked for` is somebody saying *that one*, and `found on
the network` is the network offering this machine. In both, what answers is legitimately the machine.
`remembered` is neither: it is the device's own guess from yesterday, and a guess may not rewrite the
fact it was a guess about. A record with no id yet is not anchored to anything and always takes what
answers. That is how a first connection learns its machine at all.

The cost is one bounded case. A machine reinstalled at the same address gets a new id, and this remote
keeps a dead one. The remote goes on working, because the address is still right. It adopts the new
identity the moment somebody presses Rescan or types the address.

**A record that has never connected counts as stale**, which is the honest reading of an address of
unknown age. While a connection holds, the timestamp is refreshed at most once an hour. Writing on
every refresh would be a write a second for an evening. Writing *only* when the address changed would
freeze it at the first connection. A machine in continuous use would then read as stale six hours
later.

## Discovery listens, rather than being asked

**One mDNS daemon is held open for the life of the process and a registry answers from what it has
heard.** The alternative opens a daemon, drains events for a fixed window and shuts it down again.
Then nobody is listening between two of those, and every caller pays the timeout on every look. None
of them ever learns anything in between.

**`ServiceRemoved` is believed and silence is not**, and this rule is the one most likely to read as a
leak. A departure marks a row absent and removes nothing. The reason is Android. The app holds the
multicast lock only between `onStart` and `onStop`, so a remote in somebody's pocket hears nothing.
`mdns-sd` then duly expires a machine that is switched on and two meters away. Dropping the row would
turn *the phone was in a pocket* into *the machine went away*.

So **absence is never evidence**. An absent row stays a usable cache of an address and may never
*cause* a move, and `choose` requires a present sighting in every branch.

**Looking again means building a new daemon, not asking the old one harder.** A poke throws the
`ServiceDaemon` away and starts another, keeping the registry. A button, a resumed application and a
connection that has just dropped are all pokes. **Two cheaper things do not work.** Calling `browse` a
second time *destroys* the watcher, because `mdns-sd` keeps one listener per service type and
overwrites it. The second call closes the channel the reading thread is on, and discovery freezes
silently for the life of the process.

`verify`, RFC 6762 §10.4 record verification, asks the right question and does not help either. A
daemon binds its sockets per interface when it starts, and on a phone those sockets are exactly what
has gone stale. **A socket that is no longer receiving cannot be asked more politely.**

**The registry is a type apart from the daemon, because only one of them can be tested.** Opening a
daemon binds `0.0.0.0:5353`, which `CONTRIBUTING.md`'s *No test binds a non-loopback address* forbids.
A watcher is worse than a one-shot, since it goes on retransmitting rather than shutting down after a
timeout. So `Registry` holds the merge and has no socket in it. Every test drives one by hand, and
nothing constructs a `Watcher`.

**Matching a departure to a row costs one field.** A `ServiceRemoved` carries only
`(service_type, fullname)`, so a departure has nothing to match on unless the row keeps the fullname.
The row keeps it as a *list*. Renaming a machine re-registers it under a new label while its id stays
put, and both labels name it until the old one is withdrawn.

**Time is kept in two clocks on purpose.** The registry measures in `Instant`: *how long since this
process heard from it*, monotonic and meaningless across a restart. The stored record uses
`SystemTime`, because it has to survive being written to a file. Mixing them would make every record
read as fresh after a reboot.

## `KM_NO_MDNS` declines the multicast socket, and one function honours it

**A checkout can be told to open no mDNS daemon, and nothing an owner installs is told.**
`mdns_sd::ServiceDaemon::new` binds UDP `0.0.0.0:5353` and `[::]:5353` on every interface. Windows
treats that exactly as it treats a listening TCP socket: a dialog, and a rule keyed on the full image
path. A tree being worked on relinks constantly and holds a worktree per branch. So every rule such a
prompt writes names a path that will never exist again. The dialog names a build hash rather than a
program, and answering it buys nothing.

**Binding the HTTP listener to loopback does not answer this**, and so the variable is needed. The
curation tool, the offline remote and the picture-and-bank tool all bind `127.0.0.1` unless asked for
`--lan`. All three browse for machines regardless. The socket that asks is the multicast one, and it
has no address to move.

**One function in `km_api::discover` opens every daemon in this repository**, and it answers `None`
when the variable is set. The browsing watcher, the one-shot browse and the advertisement all go
through it. So a program inherits the switch without a flag of its own, and a test inherits it without
knowing it exists. A switch honoured in three places out of four is not a switch. So
`tools/dev/check-mdns.sh` asserts the function is the only caller.

**A declined daemon and a daemon that would not start are one state**, and the honest one. A network
with multicast blocked answers an empty list, and so does this. `Watcher` already treats "no daemon"
as ordinary and lets a later rescan try again, so the switch adds no state to it. A locator that
cannot browse says so through `can_browse`. It does not offer a *Rescan* button that can only ever
report nothing.

**Set and not `0` is yes; absent or `0` is no.** `KM_LOG_FILE` and `KM_FRAME_STATS` carry the same
rule, because a third boolean with a rule of its own makes all three unguessable. The `0` spelling
lets one command ask for mDNS where the variable is set for everything around it. A shell with no way
to unset a variable for a single process needs exactly that.

**`.cargo/config.toml` sets it, and that is the line between a checkout and an install.** A release
runs from a package or a staged build and never through cargo. So a machine still advertises itself,
and a remote still finds it with nothing typed. `Taskfile.yml` sets it for what it drives. It clears it
for the two tasks that launch a staged build, because running one of those is how somebody looks at
the shipped behaviour.

## A machine has a name its owner chose, and one way in to set it

**`PUT /api/v1/machine/name` and `--set-name`, and the name takes effect without a restart.**
`machine.name` reaches the DNS-SD instance label, the `name` TXT record and `GET /discover`. Without a
route, setting it means editing JSON, which on an appliance under a television means `ssh` or nothing.

**A route of its own, not a `SettingsPatchDto` field.** The settings route carries the knobs that
belong to a *performance* and ride the 4 Hz state broadcast. A name is installation configuration that
does neither. The output device, the debugging switch and demo mode each took a route of their own for
the same reason.

**`PUT /api/v1/admin/machine/name`, so it needs the password.** It changes what the box is called on
somebody else's network and every phone in the house sees the result.

**No `GET` twin.** `/discover` is public, always, and carries the name —
which is the entire point of a name.

**Sixty-three bytes, because a DNS-SD instance name is one DNS label.** The cap is not a preference.
Relaxing it would make the advert fail on the network, where nothing would report it. It binds a second
time through `Discovery::txt_records`, whose test pins a 400-byte ceiling so the record set fits one
packet. The cut walks back to a character boundary. "Karaokê da Sala" is not fifteen bytes, and
slicing a multi-byte name at 63 would panic rather than shorten it.

**A blank name is refused on the way in and tolerated on the way out.** `tidy_name` is the writer's
rule and returns nothing for a name that is only whitespace, so an empty name is never stored.
Otherwise every client would separately have to decide what one meant. `display_name` is the reader's
rule, because refusing on the way in cannot stop a hand-edited settings file from putting one on the
wire. A client with no name shows the address rather than an empty heading. `KaraokeMachine` is
deliberately **not** filtered out there, because it is a real answer for somebody who has one machine.

There is no `--clear-name`. The way back is `--set-name KaraokeMachine`.

**Written down before it is answered to.** The controller persists, and only then does the running
machine start using the new name. So a failed write leaves the machine and its settings file agreeing.
The name is saved immediately rather than at shutdown, because a power cut does not run a shutdown
hook. A machine that came back under its old name would look like a rename that silently failed.

**The advert re-registers on a rename.** Comparing addresses only answers `Keep`. Then `settings.json`
and `/discover` both change at once, while every phone on the network goes on showing the old name
until the process restarts. The name is compared **in place**, beside the first address. It is
published twice, and a rename is a change of published fact rather than a reordering. The anti-flap
rule for the tail of the address list is untouched.

## A machine gives itself a password, and shows it on the television

**Every machine has an admin password from its first start.** It generates a six-digit PIN and hashes
it into `settings.json`. It keeps the plain text beside the hash, and draws it on the connect panel
next to the address and the QR code. Nobody sets a first password, because there is never a machine without
one.

**There is no first password to set, only one to change**, and the page that changes it is behind the
password it is changing. A machine that started passwordless would be an open one. The act that closes
it cannot be offered on the open page, or the first stranger to find the address claims the box. The
only way out of that knot is to have no open state to start from. `--set-password` on a desktop and
`POST /api/v1/admin/password` from something that already knew the address are neither of them things
a box under a television has.

**The screen is the trust boundary, and it is the right one for this machine.** Reading a code off a
television means being in the room, which is exactly the judgment `Network reach` already makes about
a home LAN. It is what a Chromecast, a smart television and a printer all do, for the same reason.

**The panel draws a PIN or nothing, and never the words "password required".** It is the *singer's*
panel, beside the QR code a phone is meant to scan. The remote that phone opens asks for no
password, because everything the door guards is under `/api/v1/admin/`, the owner's tools. A line
demanding a password there answers a question nobody in the room asked, and answers it as *you cannot
use this*. A PIN is the opposite case and belongs on the screen. It is the one fact about the machine
that exists nowhere its owner can reach, which is why this decision puts it on a television.

**`factory_pin` is the whole of what the display knows about passwords.** The bit and the plain PIN
move together, because an owner setting their own clears both in one write. So a bool beside the PIN
in `km_display::ConnectInfo` would be the same fact twice. The only branch it could buy is *on a
factory password, cannot say which*, whose honest output is nothing. `GET /discover` reports the bit,
and `/admin/` and `km-admin` nag on it; the television takes the PIN alone.

**Not a constant like `2345`.** A published default would be identical on every machine in the world
and printed in this repository. Such a machine would look closed and be open to anybody who had read
the README, which is worse than an honest open state. A PIN generated per machine is guessable only by
somebody who can see the screen.

**Six digits, never starting with zero.** Four would match the floor and be easier to read across a
room. Six give two orders of magnitude of headroom for a code that is only ever guessed through a
rate-limited login. The difference costs nobody anything. The leading digit is not cosmetic.
Anything that treats a six-digit code as a number mangles `012345`, and the code passes through
several forms on its way in.

**The PIN is in plain text in `settings.json`, deliberately and bounded.** A hash cannot be drawn on a
screen. The data directory is `0700`, and anybody who can read that file can read the hash beside it
and owns the box either way. `admin_factory_pin` being `Some` *is* the definition of "still on the
factory password". An owner setting one of their own clears it in the same write, so a machine can
never advertise a code that no longer works.

**`/discover` reports the fact and never the PIN.** `factory_password: true` is one bit, and it lets
an owner's own tools nag them. `km-admin` says so on its machine panel. `/admin/` says so on every tab
and links to the tab that changes it. The PIN itself has no business on the network; the screen is
where it lives.

**It is deliberately absent from the mDNS TXT record, and the asymmetry is the point.** Over HTTP
the flag answers a question a client deliberately asked. In a TXT record it would be shouted at the
whole segment, unasked, announcing *this machine is unclaimed*. Same bit, very different reach.
There is no `auth` record either: with a password on every machine it would be a constant.

**`discover` answers live rather than from a snapshot.** A `ConnectInfo` is built when the address is
resolved and refreshed when the address changes. A password can be set while the machine runs, and
setting one does not move an address. With a snapshot, an owner could change the password from their
phone and leave every client believing otherwise for the rest of that process's life. The routes would
behave correctly the whole time, and that is the worse half, because a client that trusts the
announcement stops asking.

**Clearing a password is resetting it.** `--reset-password`, the Machine tab's button and
`{"password": null}` all generate a *fresh* PIN rather than taking the door off. The API hands the new
one back, because a caller resetting remotely cannot go and read the television. The value is `null`
rather than an empty string, because a form submitted by accident with nothing typed must not be the
destructive act.

**Four characters is the floor.** The PIN this machine gives itself is six digits, and a floor above
what the product ships would be a rule it breaks itself. There is no complexity rule and there is not
going to be one. This guards a karaoke machine on a home LAN. A machine that lectured somebody about
punctuation would mostly stop them setting a password at all, and that is the state that is actually
unsafe.

## `/admin/` is the owner's page, and it is a third surface on purpose

**A page at `/admin/` with four tabs, served by the machine itself and behind its own admin
password.** The tabs are Songs, Pictures, Sound and This machine. Without the page, songs arrive by
routes that each assume a command line, a path already known or a file already in hand. The name
reaches the network with nothing able to set it.

**This is the third page and the boundaries between the three are the decision.** `/` is the singer's
remote and stays only that. `/dev/` stays exactly as it is: one hand-written file of vanilla
JavaScript driving raw routes, useful because it is unpolished. This is an end-user surface with an
end-user's vocabulary. It says *pictures*, not *wallpapers*, and never shows a route or a status code.

**It is not a fourth tab on the singer's remote**, which would give a guest's phone controls that
delete somebody's songs.

**A crate under `crates/machine/` rather than `crates/remote/`.** It is not a remote: it configures
the box rather than driving a performance. It is the machine's own surface, even though a desktop tool
serves the same markup.
[`One page set, two hosts`](distribution.md#two-admin-surfaces-one-vocabulary) makes a second host
possible without making this a remote. The folder is the other half. `crates/remote/` groups the
singer's remote's five hosts because there are five, and one crate does not earn a sixth top-level
directory.

**The handlers call the machine through a trait per tab.** The in-process implementation calls
`km_api::ops::*` and the trait objects directly. It never makes HTTP calls against the machine's own
API from inside its own process.

The exception proves it. The guard *does* go through `ApiState::authorize`, with an `Authorization`
header rebuilt from the browser's cookie, because one authorization path is the point. And the three
upload forms go through `km_api::uploads::receive`, the same function the JSON routes call. So the
size caps, the extension lists and the name sanitizing are stated once.

**The guard denies by default.** Every page here is an admin action, so there is one question: *is
this caller holding a valid token*. `is_open` is an allow-list of two. One is the login page, where a
refusal sends somebody. The other is the stylesheet, which touches nothing. Everything else demands
the password, **including a page nobody remembered to think about**, which is the safe direction to
fail.

`tests/pages.rs` drives the *nested* router rather than unit-testing anything. Reading axum's source
does not settle whether `MatchedPath` under `nest` reports the full path or the inner one. The guard
handles both spellings. Denying by default makes a wrong guess fail closed, including a third answer
a future version might invent.

**A tab is gated on what it exists to change, not on what it shows.** Gating on a *read* would leave
the whole setup page open on a machine whose owner had set a password. The page discloses nothing the
API does not disclose anyway. But a page of controls that would each be refused is a bad answer to
*whose page is this?* It reads as broken rather than as protected.

**`/admin/` redirects to `/admin`.** `nest("/admin", …)` matches `/admin` and answers **404** to
`/admin/`. The trailing slash is what a person types, and every link ending in a directory produces
it.

**The page reloads rather than holding a stream open.** No htmx, no script at all. It is slower than
the singer's remote and right here. These are things somebody does once and wants to see the result
of, not things they do forty times an evening. And it works with scripting off, on a phone browser
nobody chose. **So where this page has to ask a question, it asks with a page.**

**That is a property of the shared markup rather than of one program**, and it is stronger for it.
`km-admin` needs htmx for its long jobs, such as a bank download reporting progress. Those live in
*its* fragments, which it merges over this router. The templates in `km-admin-pages` carry no `hx-`
attribute, and a test says so. A dozen controls here rest on the assumption that "this page has no
script". The confirmations are pages, the *show every spelling* switch is a link, and the settings
strip is `:checked ~` sibling selectors.

**A host says which scripts its own pages load, and the machine says none.** `Admin::scripts` is
empty here, so nothing this page serves carries a `<script>` at all. The test asserts that of seven
**rendered** pages, not of the template files.

**A file scan cannot see a tag that is missing.** A host that stops linking htmx breaks nothing a scan
reads: its pages render, its tests pass, and its progress bar stops moving. So there are two tests. The scan looks for an `hx-` attribute a template
might gain. A rendered-page test looks for the script a page must not have and, on the other host,
must have.

`shell.html` emits the tags. It is the page a host renders its own body into, and the layout every
page shares does not emit them. So the tabs this crate draws are script-free on *whichever* host
serves them, which is stronger than this decision asks and costs nothing.

**No setting turns it off.** The singer's remote has `api.serve_remote`, and this has nothing. An
owner should not be able to switch off a machine's own configuration surface and then need it. The
password is the control that means something.

**The banner nags rather than warns, and it links to the Machine tab.** It says that the machine is
still on the PIN it made for itself. It is on **every** tab. Mentioning it only on the one somebody
may never open would hide the most important thing about the page they are using.

**Three controls live on the Machine tab**: changing the password, ending every session at once, and
the debugging switch. The middle one is there because *sign out everywhere* and *change the password*
are different acts. An owner whose phone went missing should not have to pick a new password and then
tell the house what it is.

### The singer's remote links to it, at the foot of the Setup tab

**Without a link, an owner has to know to type the path.** The QR code the television draws goes to
`/`. The connect panel deliberately says nothing about passwords. Only people who already know a page
is there can reach it, and those are whoever read the README. That is not the same set as whoever owns
a machine.

**So the online remote's Setup tab carries one link**, behind `Capabilities::owner_page`. It is on in
that mode and off in the offline app, which is `song_book`'s arrangement for `song_book`'s reason. The
machine serves that page and `km-remote-pages` does not. So the link is same-origin here and would
point at nothing there. The offline app also spends half its life talking to a switched-off box, and
a door onto that is a bad thing to add. Where reaching that machine matters,
[`The machine card links out to the machine's own remote`](remotes.md#the-machine-card-links-out-to-the-machines-own-remote)
already does it.

**This does not reopen `It is not a fourth tab on the singer's remote`.** That refusal is about
*controls*: a guest's phone must not hold a button that deletes somebody's songs. This is a link. A
tap reaches the login page, and every route behind it demands the token, linked from here or not.
**Being unreachable is not the protection, and treating it as one makes the page unfindable by the
person it is for.** The singer's remote stays the singer's, because nothing on it *does* anything an
owner does.

**Last on the tab, and that placement is the decision.** Setup's groups run machine, then this
device's preferences, then the rarest thing. For most people holding this phone, the owner's page is
the rarest thing on it. Leading with it would give the tab's most prominent place to the one row
fewest viewers want. The page's whole shape says
[`Setup is a fourth tab, and it is the narrow one`](remotes.md#setup-is-a-fourth-tab-and-it-is-the-narrow-one).

**The sub-line says the password is wanted.** A link that can only ever end in a prompt should say so
before the tap rather than after it. The words are the machine's own: *Set this machine up* against
`admin-title`'s *Setting up the karaoke machine*. That follows the tie-break
[`Two admin surfaces, one vocabulary`](distribution.md#two-admin-surfaces-one-vocabulary) gives.

## The owner's page takes files, and three routes carry them

**`POST /admin/packages/upload`, `POST /admin/wallpapers` and `POST /admin/audio/soundfonts`.** These
are new paths rather than a content-type branch on the routes beside them. `POST /admin/packages`
means "install what is at this path on your own disk". A package builder sharing the machine's
filesystem says exactly that. If one path meant two things to two clients, its permission and its
meaning would both depend on what the body turned out to be.

**Installing and uploading are both admin**, because both write to the machine's disk. The first names
a file the caller must already be able to put there, and the second *puts* one there. That finer
distinction mattered only while `km-package-builder` had nowhere to keep a token, and it holds a
password. A curator who does not know the machine's password pays the cost, and the cost is a
password box rather than a wall.

**No `accept_uploads`-style setting beside these three.** Debugging mode gates the two `debug/play-*`
routes, which are public when they exist. These are admin whenever they exist, so on any machine they
are already behind the password. A second switch would be one more thing meaning almost the same
thing.

### A file that is too big is a 413 that names the limit

Every one of these routes reads its body through axum's `Multipart`, so a `DefaultBodyLimit` trip
arrives as a `MultipartError`. Formatting one with `Display` produces the fixed string *"Error
parsing `multipart/form-data` request"*, whatever actually went wrong. An 85 MB package comes back with
exactly that, for a file that is perfectly well formed and simply too big.

**`status()` and `body_text()` are the accessors that tell the cases apart**, and one function uses
them. A length trip gets a **413** carrying the code `too_large`. It also carries this machine's own
sentence: *"that is larger than this machine accepts — the limit is 64 MB"*. Axum's own words are *"Request
payload is too large"*, and they name no number. Everything else keeps its 400 and gains multer's
real message in place of the fixed one.

**The limit is quoted from one function**, `uploads::limit_in_words`. `km-admin` prints "Up to 64 MB."
above its file chooser from it, and the machine refuses with it. So a page and a refusal cannot come
to describe one number two different ways. The ceiling is rounded **up**, so 2 GiB less a byte is
"2 GB". A limit quoted as a smaller number than the thing it refuses is worse than no number at all.

### What a chooser offers is quoted from one function too, and the picture chooser offers a camera roll

**`uploads::accept_for` composes the `accept` attribute for all three kinds, on both admin
surfaces.** The argument is the same as for the limit above. The list a person picks a file from and
the list their file is then measured against are one list. Otherwise they are two lists that will
eventually disagree.

**A copy agrees only until somebody changes the table.** A list typed into a template by hand agrees
exactly with `uploads::extensions_for` for as long as nobody touches the table. A fourth extension
would then be added in one place and refused in the other, after somebody had chosen the file.

**A picture also names a media type, and `image/*` leads its list.** A phone keeps photographs in a
camera roll rather than in a file system, and an extension-only `accept` reaches one unevenly. iOS
maps extensions to UTTypes and offers the photo library either way. Android's chooser decides which
view to open from the media type, and offers a file browser without one.

**The owner's page is the surface a phone actually reaches**, because the machine serves it on the LAN
to every phone in the house. So this is the difference between *Take Photo* being offered and somebody
hunting for a JPEG in Files. The order is load-bearing rather than tidy, because a chooser reads the
leading entry to decide what to open.

**The extensions stay beside the wildcard, and the wildcard does not replace them.** `.zip` is not an
image, and a wallpaper pack arrives as one. `accept` is a union, so naming both offers both.

**A package and a bank get no media type.** Neither has one a picker knows. `.kmpkg` is this
project's own, and `.sf2` is `audio/x-soundfont` at best, which nothing maps to anything. So a
wildcard would either match nothing or, widened to `application/*`, offer every file on the device.

## A page asks before it deletes a file; the API does not

**The four controls that destroy a file ask first, and they ask on the pages, never on the route.**
The four are removing a package, a SoundFont bank and a picture, and deleting a package the machine
refused. `DELETE /api/v1/packages/{id}`, `DELETE /api/v1/audio/soundfonts/{id}` and
`DELETE /api/v1/wallpapers/{id}` answer plainly: a 200 is a 200. A JSON route that asked a caller to
confirm would be lying about its own contract. Every script would have to learn a second step. The page is where a person is, and a person is what a confirmation is for.

The fourth control has no route at all, as
[`The Problems tab`](#the-problems-tab-and-deleting-a-package-that-never-installed) argues. That
changes nothing here: it is a page, so it asks.

**The guard is owed rather than optional, because an uninstall deletes the `.kmpkg`.** That file may
be tens of gigabytes and may be the only copy. The bank route has the same shape and deletes 301 MiB on
Android, where no file manager can reach the folder.

**Scoped by a test rather than by a count.** Removing a picture qualifies by meeting that test, not by
being similar. It deletes a file, and on the appliance that file cannot be put back by any means the
box has. Deleting a refused package qualifies on exactly the same terms, and more sharply. It is a
`.kmpkg` that may be tens of gigabytes and may be the only copy. Being unreadable to *this* build does
not make it unreadable to the machine that produced it.

Nothing else on `/admin/` asks twice: moving a package's bank, showing the next picture, setting the
name. **Resetting the password** is destructive and is not a file, so it stays out of the rule. It
carries the guard its own risk earns, an explicit `clear=yes` field. So a form submitted by accident
with an empty box cannot throw away a password somebody chose. **Proportionate guards, not uniform
ones**: a confirmation where a file dies, an explicit intent field where a setting does, and color
alone everywhere else. A page that asks twice teaches people to click twice.

**Each surface asks in the way its own decision commits it to.** `/admin/` gets a **server-rendered
confirmation page** at the same address the `POST` goes to, with `GET` asking and `POST` answering.
That page carries no script at all, so `hx-confirm` and `confirm()` are both unavailable. A
confirmation is the one thing on that surface that must not need JavaScript. `/dev/` gets a one-line
`confirm()`, native to one hand-written file of vanilla JavaScript.

**The confirmation is a courtesy, not the enforcement.** The `POST` refuses on its own. A token or a
`confirm=1` field would invent a second enforcement point that the API does not have and `/dev/` would
not send. And nothing here can change under the confirmation, unlike the package builder's bulk edits,
where a *filter* can.

**Both pages name the file's size**, so the guard can tell 34 KiB from 20 GB. The page reads the size
at the moment somebody is asked, and never stores it or shows it on the list. A stored number goes
stale, and a `metadata` call per row would put filesystem touches on a read path that has none.

### A control that can only be refused is left out, not grayed

**`PackageDto::removable` and `SoundFontBankDto::removable`, and a page spends them by omitting the
control.** Both routes refuse permanently in two cases: a file `debug.` names, and a file outside
every folder the machine owns. Neither becomes allowed by waiting. A control that is always refused
teaches somebody to ignore the row it is in. `AudioOutputs::changeable` is the precedent for
**having** the flag, not for how it is spent. A device that cannot be changed now can be changed when
the song ends.

**One predicate, asked twice.** The machine answers "may this go?" with the *sentence* its refusal
would carry, and the route and the page call the same function. So a page that leaves a button out
leaves it out for exactly the reason the route would have given. A bare `bool` would leave the page
inventing wording for a rule it does not own.

**The sentence stays in the machine and the boolean goes on the wire.** Every reason names a full
path. The directory layout of the machine under the television is nobody's business but the owner's.
The same rule keeps an archive's path out of `PackageDto`. `/admin/` runs inside the
machine and prints the sentence; a remote gets the flag.

### `DELETE /packages/{id}` answers three failures three ways

Mapping every error with `|_| not_found(…)` is the trap. A package the machine refuses to delete and a
disk that would not release the file would both come back as `404 package '<id>'`. That is the one
status that says *it is not here*, false in both cases, and it throws away the machine's own account
of why.

Not installed is a **404**, and refused is a **400 with the sentence**. A file that could not be
removed is a **500** saying nothing was uninstalled. A client treating 404 as "already gone, fine"
sees the difference, which is the point.

## The Problems tab, and deleting a package that never installed

**A fifth tab on `/admin/` gathering everything the machine found and could not use, and the one
control on it deletes a refused package's file.** `A clash warns rather than only logging` makes a
refusal audible on three surfaces, and none of them can act on it. The `.kmpkg` sits in the folder
and is refused again at every scan. It goes away only through the box's filesystem, and an appliance
under a television has no way to reach that. A refusal audible three times over is one somebody has to
be able to act on.

**`uninstall` structurally cannot reach these**, so this is a new operation rather than a new caller
of an old one. It looks a package's path up from its catalog rows, and a package that would not open
has none. So `DELETE /packages/{id}` answers 404 for exactly the files this is about.

**The tab is always in the bar, never conditional.** Three reasons, in ascending order of force:

- A tab that comes and goes is a navigation that changes shape under somebody.
- A report that disappears when it is clean is how faults get missed.
- Deleting the *last* problem would make the tab vanish while you are standing on it. The redirect
  afterwards would then point at a tab the bar no longer has.

A count badge earns the tab its place on a healthy machine. The **chrome** carries it, so it shows on
every tab, because a badge is only useful to somebody who opened a different page.

**Delete, and none of the three obvious neighbors.** A *rescan* button would imply the machine was
waiting to be asked, but it retries every file at every start and through `POST /packages/rescan`.
*Replace it in place* means writing into a file this machine has already refused, under a name chosen
by whoever made it. *Download it back* publishes an unreadable archive over the LAN. What is left is
the one thing a browser can usefully do to a bad file. Where there is a better answer, the
confirmation states it, and the answer is to rebuild the package and send it.

**The row names the folder, where every other surface names only the file.** That reverses the rule
`A clash warns rather than only logging` sets for the banner and the idle screen, and it has to. Take
the same package in two of the scanned folders. It produces two rows with identical text and two
different Delete controls, which is worse than no page. The path is publishable here for the reason
`A control that can only be refused is left out, not grayed` already gives. `/admin/` runs inside the
machine and prints paths in its refusals; a remote gets a flag.

**A derived id addresses a refused package: the file's name, then a fingerprint of the whole
path.** It is not a field on `PackageProblem`. The path is also the key a problem is remembered under,
and a stored id would be free to disagree with it. It is not the name alone, because of the
two-folders case above.

It does not round-trip. A delete matches it against a freshly read list and never joins it onto a
folder. So an id a browser invents can only ever miss.

Two files answering to one id delete **neither**. A collision needs 32 bits to land and will very
likely never happen. That is exactly why the branch is written, rather than left as a `find` that
would silently take the first.

**Deleting the file releases the bank it had reserved.** This is the second exception to *a package
that merely stopped being found keeps its reservation*. `install` deliberately calls
`ensure_bank` before indexing, so the answer survives an install that then fails. A package that
opened and failed afterwards therefore holds a thousand numbers it has no songs in. Nothing else would
ever give them back. The argument is `uninstall`'s own, word for word: deleting the file is the owner
saying it is not coming back.

The rule is intact. It is about a file that is still somewhere, and this is about one the machine has
just destroyed.

**The tab reports more than packages, and every row carries the control that fixes it.** The sound
fault, the audio device having fallen back to a second choice, and the picture fault are each a
sentence the machine already composes. Without this tab, two of the three appear on no tab at all. A
download in progress is deliberately **not** here. This tab is for standing faults the machine is
still holding, and without that line it would accrete every transient toast.

### Every fault on the Problems tab draws its own control

**A page that gathers faults it cannot fix is the one failure mode this arrangement has that per-tab
notices do not.** So each row carries the control that fixes it: a bank picker, the output picker, a
file chooser for an empty rotation. A link to the owning tab is not enough on its own. The device
fault means an appliance is playing to nobody after its ALSA card order moved between boots. Its link
goes to Sound, which lists banks and has nothing to do with outputs.

Three properties are load-bearing:

- **They post to the owning tab's own routes.** A fix applied here and the same fix applied on Sound
  are one act. A second route would be a second thing to keep in step.
  `POST /admin/packages` and `POST /admin/packages/upload` are split to avoid that mistake in
  reverse. `POST /admin/sound/{id}/use` is `POST /admin/sound/use` with the id in the body. A
  `<select>` cannot feed a path segment on a page with no script.
- **A `back` field brings the redirect home**, whitelisted to a fixed set of tab names. The value
  comes off a form and reaches a `Location` header.
- **The link stays beside the control.** The owning tab knows things this one does not: sizes, where
  the pictures come from, every spelling of an output. So it earns its place beside the fix rather
  than in place of it.

**The refused-package rows keep their rule, including the ones that offer nothing.** A file under
`debug.packages`, or outside every folder the machine owns, still shows the machine's refusal in
place of Delete. See
[`A control that can only be refused is left out, not grayed`](#a-control-that-can-only-be-refused-is-left-out-not-grayed).
A machine deleting a file it does not own is a different decision from this one, and nobody has made
it.

**Nor is there a rescan button or a delete-everything button.** The rescan argument above holds.
A single confirmation in front of an unbounded number of deleted files is not the guard that rule
asks for.

**The wording of the sound fault lives in `SoundFontStatus::complaint`** and is not restated here.
The distinction it encodes is easy to get wrong in a way that matters. `problem` means *there is no
bank* and reads as a broken machine; `fallback` is a caveat about one working perfectly well. A second
copy would have the television and the browser describing one machine two ways.

**No API route, and this is the only control on `/admin/` without one.** `packages.uninstall` is the
permission, because the act is *delete a `.kmpkg` this machine found*. A new id would leave an owner
who had locked that one still holding a file-deleting control under a name they had never heard of. A
route would let a script do what a person can, and no client wants that:

- The singer's remote must not have it.
- `km-admin` sends files and never deletes.
- A developer at `/dev/` has a shell and can delete the file, which is the whole condition this
  feature exists to relieve.

**The cost is real and is the price.** An owner who cannot reach `/admin/` has no route to this. Their
remedy is the one that always existed, and this removes nothing. Adding the route later is cheap,
because the operation is on the `Catalog` trait either way. A published route is a contract, and it
would then constrain the id's shape by compatibility rather than by what the page needs.

## Changing the admin password is something a phone can do

**`POST /api/v1/admin/password`.** The appliance has no keyboard for a command line. The password on
a fresh machine is a PIN somebody read off a television. So the act of replacing it has to be
reachable from the thing they are holding.

**Under `/api/v1/admin/`, so changing the password needs the current one.** The route that grants
permanent control of the machine must not be reachable without it.

**Every outstanding session ends on a change**, including the browser's own, and the page says so
rather than leaving it as a surprise. It is a property of the construction, not a step somebody has to
write. A token is an HMAC keyed on the stored hash, so a different hash cannot produce the same MAC.

**`null` resets rather than clears**, and the reply carries the new PIN. There is no state with no
password to clear it *to*. What an owner means by that button is "I have forgotten mine". So the
machine invents a new one and puts it on its own screen. It hands it back here too, because a caller
resetting remotely cannot go and read the television. The value is `null` rather than an empty string,
because a form submitted by accident with nothing typed must not be the destructive act.

**Four characters is the floor and there is no complexity rule.** See
[`A machine gives itself a password, and shows it on the television`](#a-machine-gives-itself-a-password-and-shows-it-on-the-television)
for why four and not eight.

### The Machine tab changes a password, and there is no first one to set

**A Change box and a Reset button, always.** Reaching this page takes the password. No state exists in
which a Set button on a page anybody could open hands the machine to whoever pressed it first.

**Reset, not Remove.** The destructive act is going back to a freshly generated PIN, which the
television then shows. It carries a `clear=yes` field: it is destructive, and a form submitted by
accident with an empty box must not do it.

## Turning demo mode on is an owner's act; knowing it is on is not

**`PUT /api/v1/admin/demo` is behind the password, and `GET /api/v1/demo` is not.** Turning demo mode
on makes the box **start playing music by itself, indefinitely, in somebody's house**. With `persist`,
it goes on doing so after the power is cut and restored. A guest with the address should not be able
to turn that knob. Nobody should have to work out how to undo it.

Reading it stays public. The debug play routes are public while their switch is not, for the same
reason: **using the answer and changing the state are different acts.** A remote has to know that a
demo song is on the deck. Only then can it tell a singer that this one will not run out on its own:
the next one starts the instant it ends. So what gets the singer a turn is queueing, which takes the deck off a
demo at once. The answer says nothing a person in the room cannot already hear.

**The switch is for the run unless it is asked to persist.** The request body carries that choice;
there are not two routes. `{"enabled": true}` changes the running machine and nothing else. Adding
`"persist": true` also writes `demo.enabled` into settings.

The un-persisted form is the default, because it is the smaller surprise. A party is a run, and
somebody who switched the machine on for an evening should not have to remember to switch it back. A
switch that turned out to be permanent is discovered a week later.

A persisted change is written to disk immediately rather than at shutdown. Somebody is standing in
front of the machine saying what it should do when they are not there. A power cut before the next
clean stop must not quietly undo it.

**Switching demo mode off does not stop the song that is playing.** It is a mode, not a transport
command, and `POST /api/v1/transport/stop` is the thing that means stop.

**The owner's act has an owner's control, on the owner's page.** *This machine* in `/admin/` carries
both boxes, one for tonight and one for after a restart. `km-admin` carries the same two under the
same words. An admin-only route reachable from nowhere an owner goes is a feature nobody can use.

**Two boxes and not one**, because the route takes two answers. A page that hid the second could not
express the state the route can: *on tonight, off tomorrow*. The page leaves `persist` unticked by
default, for the same reason it defaults to `false` in the body.

**The singer's remote cannot reach it.** That page has the one-shot *play something* button and
never the mode. See
[`The remote can ask for a demo song without turning demo mode on`](interface.md#the-remote-can-ask-for-a-demo-song-without-turning-demo-mode-on).

**The machine's own keyboard reaches it, and that is not a hole in this.** `D` at the machine turns
the mode on and off without a password. See
[`Demo mode has a key on the machine's own keyboard`](interface.md#demo-mode-has-a-key-on-the-machines-own-keyboard).

This route gates somebody who found the box *on the network* making a room they are not in play music
indefinitely. Whoever presses the key is standing in that room. What the key cannot do keeps the two
consistent: it never persists. So the answer to "what should this machine do when nobody is here?" is
still only ever given through the password.

**The admin page's row needs no mark of its own.** Every page there is behind the password, so the
row that turns demo mode on is gated exactly as the rest of the tab is.

## The demo delay is a route of its own, and it is always written down

**`PUT /api/v1/admin/demo/delay` takes `{delay_secs}` and nothing else.** A plain number in
`settings.json` needs nothing discovered before somebody can name it. That test kept it off the API,
and it is the wrong test on the machines this is for. **An appliance under a television has an owner's
page and no text editor.** A delay nobody can reach is a delay every house gets, whether it suits them
or not.

**A path under the switch rather than a third field in its body, and `persist` is the reason.** `PUT
/admin/demo` is shaped around *for tonight or for good*, because a party is a run. A delay is
installation configuration, in the family of the machine's name and its locale. It is **always
written to settings**. A body where `enabled` obeyed `persist` and `delay_secs` ignored it would make
one flag mean two things. Two forms with two Save buttons is what that costs on the page.

**Admin, for the switch's reason and one of its own.** The smallest legal delay is zero. Somebody who
could set this could arrange for a house they are not in to sing the moment it goes quiet. That is
[`Turning demo mode on is an owner's act`](#turning-demo-mode-on-is-an-owners-act-knowing-it-is-on-is-not)
reached by another door. Reading stays public on `GET /api/v1/demo`: using the answer and changing the
state are different acts.

**An hour is the cap, and it bounds the route rather than the setting.** Past that, nobody can tell a
demo from a machine that never performs. So a larger number is likelier a typo than an intention, and
what sends one is a box on a page reached from a phone. `settings.json` is uncapped, because somebody
editing their own machine's file is being deliberate. Over the cap is a **400 rather than a clamp**. A
machine that quietly stored a number nobody chose is worse than one that said no.

**Changing the delay shifts the running deadline rather than re-arming it.** The deadline is *when
somebody last did something, plus the delay*, so a new delay measures from that same moment. Quiet for
fifty seconds and told to wait sixty leaves ten to go; told to wait thirty, the next poll starts a
song. Re-arming from now would make *shortening* the delay lengthen the wait, once. That would happen
in front of whoever had just shortened it to find out whether it worked. It is
[`the clock counts idleness`](interface.md#what-the-machine-does-when-nobody-is-singing) from the
other side.

**`min_suitability` stays off the API.** It is the one demo key a person cannot judge from the room.
A delay is a length somebody feels, where a floor of 5 against 6 is a claim about what a packager
measured. Choosing it wants the catalog in front of you rather than a phone.

## Starting one demo song is anybody's; turning demo mode on is not

**`POST /api/v1/demo/start` is public**, beside a `PUT /admin/demo` that is not. The difference is one
song against a mode. The pair to compare it with is one screen over: advancing the picture is public,
and adding one to the rotation is not.

**Three things make it a smaller act, and all three are enforced rather than argued.** It is *refused*
unless the deck and the queue are both empty. So it interrupts nobody and takes no turn away, where a
skip over a song does both. It does **not** require demo mode and does not turn it on. Chaining is what
`demo.enabled` buys, so with the mode off this is exactly one song and then silence. And it writes
nothing.

**The mode is what keeps this route worth having beside `POST /transport/skip`.** That route reaches
the same flag on an empty deck by
[a rule of its own](interface.md#skip-into-silence-asks-demo-mode-for-a-song), and that press needs
the mode on. This route exists for a machine whose owner never turned the mode on. There, it is still
the only way to ask.

**A trigger that first needed the admin password would be useless to the room it exists for.** The
feature's whole argument is that somebody should be able to hear what the box holds without first
working out how to drive it.

**It answers `no` synchronously and answers `yes` on a promise.** The three refusals are something
loaded, something queued, and no sound at all, and they are knowable without touching the catalog. So
they come back as a 409 `unavailable`, carrying a sentence a remote can put on screen unchanged. A *fourth*
possible failure is deliberately not among them: a catalog with nothing playable in it. Only the two
full-table draws the picker makes can discover that. This route exists to keep exactly that work off a
request thread, and a machine with no songs says so on every other screen.

**The route sets a flag; the poll thread starts the song.** Every demo start already happens on the
one poll thread. That makes the check-then-start there safe with no lock spanning the load. Starting a
song from a request thread would race that check. With two starts, the second load cuts the first song
off a few milliseconds after it began. So the press sets `demo_once`, and the next poll acts on it
within fifty milliseconds, with one writer throughout.

**The flag is not the deadline moved forward, and that distinction has a cost attached.** Setting
`demo_resume_at = now` looks equivalent and is not. Take a machine where the mode is on and somebody
skipped ten seconds ago. A trigger that then found nothing to play would also cancel the minute of
silence that skip bought them.

A separate one-shot leaves the ordinary clock alone. The attempt it causes spends it, including an attempt that found nothing, and that stops an empty catalog being
retried twenty times a second. Anybody who queues, skips or stops first cancels it.

**The body describes the machine as it stands, not as it will be.** `playing` is false in the answer
and `starts_in_secs` is `None`, because the song has not started yet. What a caller wants from it is
`enabled`. That tells them whether another song will follow the one they asked for.

## A refusal travels as a code, and whoever shows it writes the sentence

**The machine composes in English because it has no idea who is reading.** So a remote that renders
the `message` beside a 409 puts an English sentence inside a Portuguese page.

A refusal a *singer* can provoke therefore carries a stable code in the `error` field. The surface
showing it looks the sentence up in its own catalog: `no_key_video`, `nothing_playing`, `no_melody_channel`, `no_sound`.
**A remote renders the code and never the `message`**, which stays for the log and for a JSON client
with no catalog.

**A refusal only the *owner* can reach carries no code.** Examples are a SoundFont that will not load,
a path outside the allowed roots, and an upload that could not be written. Those are diagnostics read
beside a log, and a code per case would say less than the sentence does.

**The kind of song rides in the code**, so there are three of each rather than one. English writes
`a video song has no key`, and Portuguese writes `uma música em vídeo não tem tom`, with an article
that agrees with the noun. A sentence the machine already composed cannot be taken apart again.
Reading the kind back out of it would be a parser for prose the API documents as unstable.

**An unknown code is not an error.** It renders as the generic refusal. An offline remote talking to
a machine of another version shows a plain sentence, not a blank, a shrug, or a red failure. The two vocabularies are deliberately not one dependency. The machine links the pages, so the
codes are spelled on both sides, and a test in the machine stops them drifting.

**A 400 gets no code.** It says *fix what you sent*, so it is aimed at whatever built the request, not
at the person holding the phone. The remote clamps a key change before it asks, so a singer never
sees one.

## Power is a capability of the host, not a method on the machine

There are three power routes: `GET /api/v1/admin/power`, `POST /api/v1/admin/power/off` and
`POST /api/v1/admin/power/restart`. They are **mounted only on a host that can do something about its
own power**. A machine that cannot answers a real 404 on all three.

**That follows a rule `Controller` already states rather than inventing one.** That trait is the
things every host can do: playback, the queue, the settings that belong to a performance. It has no
defaulted methods by decision, because a default would be a lie a test double then tells quietly. Its
own note says what to do with a capability that genuinely varies:

> *"the honest shape is a route that is **not mounted**, so a machine without debugging answers a real 404 rather than a 409 about a method that quietly did nothing."*

Powering a box off varies exactly that way. An Android television cannot, a desktop must not, and
only a supervised appliance both can and should. So it is a separate seam, and `km-api` implements
none of it. The binary crate supplies the implementation, the same division the catalog and the audio
device already have.

**They are admin routes, where the debug pair is public, and the asymmetry is not an
inconsistency.** Debugging is a *mode* somebody switches on, so its routes are absent or public.
Power is a *capability* the host has or has not, so its routes are absent or the owner's. Both obey
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission). One thing must never happen:
a power route outside `/api/v1/admin/`, which would let anybody on the LAN switch the television off.
It has an assertion of its own, beside the mirror-image one for the debug pair.

**`GET /discover` says nothing about this.** It is public. A field there would tell the whole network
that this machine can be powered off remotely. Nothing needs to know that before it has a password.

**Shutting down asks the operating system and stops nothing itself.** The supervisor notices the box
going down and stops the unit the way it stops it for any other reason. That runs the one shutdown
path that already exists and is already tested, byte for byte what the physical power button does.
Setting the machine's own shutdown flag *as well* would race that. The process would begin persisting
settings and dropping the audio device while systemd was separately stopping it. The two orderings
would interleave differently every time.

**Restarting is an exit, not a request to the supervisor**, and that is forced rather than chosen.
Asking systemd to restart a unit is `org.freedesktop.systemd1.manage-units`, which an unprivileged
account does not get. Exiting needs no privilege whatsoever, and `Restart=always` does the rest. So
the route sets the one existing stop flag, and the ordinary shutdown block runs. There is still one
exit and not three.

**The availability gate is `INVOCATION_ID`, not "would the operating system allow it".** Those come
apart exactly where it matters. Take a developer running the binary from a terminal on a Linux
desktop. They are inside their own active logind session, so logind *would* switch their desktop off. Offering that is
the same category error as `systemctl enable` in a `postinst`.

systemd sets `INVOCATION_ID` in every unit's environment, and a `cargo run` never has one. So it
answers *"I am supervised"*. That is also the question a restart needs answered, since exiting only
starts the process again if something is watching. One capability covers both actions rather than two
flags, because the two conditions coincide.

**Deliberately not probed: whether the session is Active and whether polkit will allow it.** Both are
true at one moment and false at the next, when somebody switches virtual terminal. Both come back from
`systemctl` as a readable sentence. Reporting them as *unavailability* would hide a control because of
a condition that no longer holds.
[`A control that can only be refused is left out, not grayed`](#a-control-that-can-only-be-refused-is-left-out-not-grayed)
draws the line at *permanent*.

**Both answer `202`, and mean it.** The box has not powered off when the body is written, and the
caller will get no later word from a machine going dark. That is the literal case the code exists
for.

So a refusal cannot be a status. It arrives after the answer has gone, as a journal line carrying the
operating system's own sentence. *"Interactive authentication required."* is something somebody can
search for; "power off failed" is not.

## The machine's own log is a route, and it is the owner's

`GET /api/v1/admin/logs` answers the last few hundred lines the machine said. `GET
/api/v1/admin/logs/stream` sends those and then the ones that follow. The machine keeps them in a
bounded ring in memory. A third `tracing` layer fills it, beside the console and the file.

**What this reaches is the machine whose log is hardest to get.** A box under a television has no
console and nobody logged into it. A run started by double-clicking its icon on Windows has a null
standard output handle and discards every line. A file means finding the folder, over SSH or `adb`,
after the moment has passed. This is the same stream at a third destination, and a person already has
that destination open.

**Admin routes, like power and unlike the debug pair.** A log line names the file the machine opened
and the folder it scanned. It names the address it resolved, the audio device it found and the name
its owner gave it. Reading that is the owner's business, and
[`The URL prefix is the permission`](#the-url-prefix-is-the-permission) leaves no third state to put
it in.

**They carry paths on purpose, where the rest of the surface strips them.** `PackageDto` keeps an
operator's paths off the wire, and a fault's reason names a file rather than its location. Both are
right for what they serve: a row on a singer's remote spent on `C:\Users\…\` is a row wasted. A log
line without its path says nothing at all. The audience is what differs, and that is exactly why these
two are the routes behind the password.

**Which makes one standing constraint.** Anything ever written to this log becomes readable by
whoever can reach a machine with the development console on. A line that would say a password out
loud is a fault where it is written, not here.

**On the dev mirror, where the power routes are not.** That exception is about a change nobody can
undo from a page, and reading a log is not a change at all. The mirror already carries a route that
plays any path on the machine's disk to whoever asks. A tail is well inside a bargain that includes
that. So the two conditional admin surfaces part company at exactly one point, and each has an
assertion naming it.

**Absent rather than empty on a machine that keeps none.** The routes are mounted where a tap was
installed and answer a real 404 where none was. That is the shape `Controller`'s own note prescribes,
and the one the power routes already take.

**The tap is unconditional, where the file beside it is asked for by name.** That is not a
disagreement with [`A log file for the runs nobody is
watching`](distribution.md#a-log-file-for-the-runs-nobody-is-watching). It is the same argument
reaching a case where the price is different. A file costs a directory that fills up, so somebody
decides; a bounded ring costs a fixed few hundred kilobytes, so nobody has to.

And the run that most
needs a log is always the one nobody armed. By the time a person wants the last hundred lines, it is
too late to begin keeping them. Holding them publishes nothing, because the routes govern reading
them.

**The verbosity ladder still decides what goes in it.** That decision settles this half outright: a
level says how much detail, and where the detail goes is a different question. `-v` and `RUST_LOG`
reach the ring exactly as they reach stdout. The directive in force travels with the tail. A machine
started without `-v` keeps nothing below `info`. Without the directive, a pane with no `debug` lines
in it is indistinguishable from a machine with nothing to say.

**A record is structured, not a formatted line.** The level, the target, the message and the fields
arrive apart, so a reader can colour by one and filter by another without parsing text. The time is
milliseconds since the epoch, and nothing formats a clock on the machine's side. Whoever draws it has
a locale, and this has no date library to get one.

**A stream carries frames, and falling behind is a frame of its own.** The alternative is a synthetic
record saying so, and that forges a line the machine never emitted. Once somebody pastes the pane into
a report, it is indistinguishable from a real one. A reader hiding everything below `warn` filters it
away. The event stream draws the same distinction for the same reason.

**A reader gets a tail and a subscription that overlap**, and drops the overlap by the sequence
number every record carries. The ordering that cannot duplicate loses the records taken while a
reader is arriving. That is exactly when a machine is busy enough to be worth watching. A duplicate a
reader can see and discard beats a gap nobody can.

**Nothing serving this may say anything.** The tap takes a line emitted while feeding a reader and
sends it to that same reader. On the arm that reports falling behind, it feeds the reader least able
to keep up.
