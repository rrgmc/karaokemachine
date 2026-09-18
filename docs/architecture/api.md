# The HTTP API

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

`axum` + `tokio`, JSON. **Default bind `0.0.0.0:8177`**, because a phone acting as the remote is the
intended use and a machine nobody can reach is the broken case rather than the safe one.

**`km_api::routes::SURFACE` is the route table in code**, and it is the index — this file does not
keep a second copy, because a hand-written list of fifty routes is a list that goes stale between the
day it is written and the day somebody trusts it. Three tests hold the surface to `SURFACE`: one
driving a request at every entry, one asserting that every path under `/admin/` refuses a tokenless
caller and every path outside it does not, and one checking the table's own shape. So the answer to
*what routes are there?* is one `grep`, and the answer to *is that still true?* is `cargo test`.

The areas it covers: discovery and connect, songs (search, one song, lyrics, export, the printed
book), the queue, transport, settings, mics, audio outputs and SoundFont banks, wallpapers, the
machine's name, demo mode, packages, admin login, password and sessions, the debugging switch, and a
WebSocket at `/api/v1/events`. `DEBUG_SURFACE` holds the two routes mounted only in debugging mode.

**`GET /api/v1/demo` and `PUT /api/v1/admin/demo` are the same resource in two prefixes**, the
arrangement every read/write pair here now uses. Reading is a guest's business and switching it on is
an owner's — see the decision in
[`api-and-network.md`](../decisions/api-and-network.md#turning-demo-mode-on-is-an-owners-act-knowing-it-is-on-is-not).

**A third path sits beside them: `POST /api/v1/demo/start`, and it is public.** One song rather than a
mode — the reasoning is
[`Starting one demo song is anybody's`](../decisions/api-and-network.md#starting-one-demo-song-is-anybodys-turning-demo-mode-on-is-not).
A path of its own rather than a third method on `/demo`, because `POST /demo` would read as creating
demo mode, which is what the `PUT` already does with a body. **A new route needs nothing listed
anywhere**: which prefix it is declared under is the whole of its permission, and adding a row to
`SURFACE` is what puts it in the sweep.

**A fourth, `PUT /api/v1/admin/demo/delay`, sets how long the quiet has to last.** Under the switch
rather than a field in its body, because `PUT /admin/demo` is *for tonight or for good* and a delay is
always written down — the argument is
[`The demo delay is a route of its own`](../decisions/api-and-network.md#the-demo-delay-is-a-route-of-its-own-and-it-is-always-written-down).
It is where `Controller::set_demo_delay` and `MAX_DEMO_DELAY_SECS` live, and past the cap it answers
400 rather than clamping.

All three writes are **synchronous**, unlike every other route that can result in a song starting. `PUT
/demo` and `PUT /admin/demo/delay` move a deadline and `POST /demo/start` sets `State::demo_once`;
either way `Machine::poll` on
the `km-poll` thread starts the song up to 50 ms later, so neither needs `ops::off_runtime` — the work
that could block a runtime thread is already off it by construction. For the trigger that is not only
an economy but a correctness property: every demo start happens on that one thread, which is what
makes `maybe_start_demo`'s check-then-start safe with no lock spanning the load, and a start issued
from a request thread would race it into loading two songs. `arm_demo(DemoEvent::Somebody)` clears
`demo_once`, which is both the attempt that spends it and the deliberate act that cancels it.

`POST /transport/skip` reaches that same flag on a machine with an empty deck and the mode on, which
makes it the fourth write that can end in a demo song starting. `Machine::transport` arms `Somebody`
before the match and sets the flag inside it, and the order is what the arrangement rests on: the arm
clears a pending trigger, so a flag set any earlier would be wiped by it.

The wire gained `OriginDto::Demo { number }`, a third variant on `now_playing.origin` carrying no
`entry_id` because a demo song was never queued. Every client in the workspace parses `km_api::dto`
types directly rather than mirroring them, so the addition is compile-checked rather than a silent
parse failure somewhere.

**A route may have two widths and one id.** `GET /audio/soundfonts` answers with the nine banks the
machine offers, or with the whole sixty-odd-row survey when asked `?all=true` — same `audio.read`, no
second entry in `SURFACE`, since what changes is how much of one answer is drawn rather than what a
caller is allowed to do. Query parameters are `Deserialize` structs without `deny_unknown_fields`, so
a client built against a later version can pass a key this one has not heard of and still get its
answer.

## Admin mode: one prefix, one predicate

**The permission is the URL.** `routes::needs_admin_token` tests the request path against
`/api/v1/admin/` — with the trailing slash, so `/api/v1/adminfoo` is not one — and exempts
`admin/login`. `ApiState::authorize` is its only caller and takes nothing else: not the route, not
the peer address, not a map.

**One middleware, on the *outer* router, and the placement is the only one that works.** Inside
`nest(API_PREFIX, api)` axum has already stripped `/api/v1` from the path a layer sees, so a
middleware there would test `/admin/demo` against a rule written about `/api/v1/admin/demo`. Out here
the path is whole. It is harmless to the HTML pages, which live at `/admin/` — outside the API prefix
— and carry their own guard.

This replaced a 46-entry route-id map and three rules that overrode it. What the map cost was a table
that could disagree with the router; what it bought was a configurability nobody used. The decision
is [`The URL prefix is the
permission`](../decisions/api-and-network.md#the-url-prefix-is-the-permission).

**A machine with no password refuses every admin route rather than opening every one.** That is the
inversion: the old rule made an `admin` route public when no password was set. Settings generate a
PIN at first start so the state does not arise, and if it somehow does — a hand-edited file —
refusing is the safe answer.

### The same surface again, where nothing is gated

`router_with` mounts `api_router` twice: under `API_PREFIX`, and — while
`routes::dev_console_served` says both developer switches are on — under `DEV_API_PREFIX`,
`/dev/api/v1`. A function called twice rather than a clone, so the code says there is one definition
of the surface and two places it is mounted.

**The middleware needs no exception, and that is the mechanism rather than a happy accident.**
`/dev/api/v1/admin/password` does not begin with `/api/v1/admin`, so the `strip_prefix` in
`needs_admin_token` misses it and the whole mirror is open. `routes.rs`' own tests sweep every
`SURFACE` entry under `DEV_API_PREFIX` and assert none of them wants a token, because a later edit
loosening that predicate — to a `contains`, say — would close the mirror with everything else still
green.

`/dev/api/{*rest}` is mounted **unconditionally**, mirroring `/api/{*rest}`: without it, a path under
a mirror that is not mounted fell through to the root fallback and answered a JSON client with the
HTML landing page and a 200.

The decision, including what bounds the cost, is [`The development console has an API that needs no
password`](../decisions/api-and-network.md#the-development-console-has-an-api-that-needs-no-password).

### The password, and the token

One shared password, hashed with **`argon2`**, unchanged by this. The salt is `argon2`'s own: since
0.6 the hasher draws it, so `AdminAuth::hash_password` does not generate one to hand in — that
upgrade also took `rand_core` out of the crate, its 0.10 line having moved `OsRng` back to
`getrandom`, where the bytes always came from. The stored string is a PHC `$argon2id$v=19$…` either
way, and `a_hash_written_by_argon2_0_5_still_opens` holds real 0.5.3 output to prove a settings file
written by an older build still lets its owner in.

**A token is not stored anywhere.** It is

```
v1.<expiry unix seconds>.<nonce>.<mac>
mac = HMAC-SHA256(key = the stored argon2 hash, msg = "km-admin-v1|<epoch>|<expiry>|<nonce>")[..16]
```

and `verify` recomputes it. There is no token table, nothing in memory, and **a restart keeps
everybody logged in** — which is what the whole shape is for.

Two properties fall out of the construction rather than being written:

* **Changing the password ends every session**, because the hash is the key and a different key
  cannot produce the same MAC. The old implementation cleared a `HashMap` to achieve this.
* **A token from one machine does not open another**, because every machine's argon2 salt differs.

**`api.session_epoch` is the revocation that would otherwise be lost.** It is inside the signed
message, so bumping it invalidates every outstanding token without touching the password — *sign out
everywhere*, for a phone left in a taxi. It is persisted, and has to be: an epoch that lived only in
memory would come back as the old one after a power cut and un-revoke everything.

**Logout cannot revoke one session, and the route says so** rather than implying otherwise. It clears
the browser's cookie; a token copied off the wire lives until it expires or the epoch moves.

**Expiry is wall-clock, and that is forced.** `Instant` is monotonic since boot, so it resets on the
restart the token now has to survive. The cost is a machine whose clock jumps keeping tokens too long
or too briefly — the appliance may have no RTC — and the epoch is the bound no clock can move.

**Rate limiting is per address *and* whole-machine.** Five failures a minute from one address was the
whole of it, and against a six-digit PIN that is now the only gate; addresses are free on a LAN, so a
per-address budget alone scales linearly with an attacker's patience. A whole-machine budget of
twenty closes that. A success forgives that address's failures and deliberately **not** the global
count: one correct login does not vouch for the hundred wrong ones that came from elsewhere.

### The PIN a machine gives itself

`Settings::load` generates a six-digit PIN, argon2-hashes it into `api.admin_password_hash`, and
keeps the plain text in `api.admin_factory_pin`. Plain text is what lets the connect panel draw it;
the data directory is `0700`, and anyone who can read that file can read the hash beside it.

`admin_factory_pin.is_some()` *is* "still on the factory password". It reaches `ApiConfig` as
`factory_password`, and `GET /discover` reports that one bit — never the PIN, and deliberately not in
the mDNS TXT record, which is broadcast unasked. **The bit does not reach the display**:
`karaokemachine::connect::to_display` carries the PIN across and drops `factory_password`, so the
connect panel draws `· PIN 482913` or nothing at all. `generate_factory_pin` refuses a leading zero:
`012345` is mangled the moment anything treats a six-digit code as a number.

**The floor is `km_api::MIN_PASSWORD_CHARS` and is read rather than repeated.** Three surfaces
refuse a short password — the JSON route, the owner's page at `/admin/`, and `km-admin` before it
sends — and three copies of `4` disagree the moment one counts *bytes* where the API counts
characters: a two-character CJK password is then stored by one and refused by the other, with
nothing anywhere reporting it. The constant lives beside `FACTORY_PIN_DIGITS`, since what a machine
gives itself and the least it will accept are halves of one policy. `minlength="4"` in the two
templates is the one place the number is still written a second time, a template being unable to
read a Rust constant; each says so in a comment beside it.

### Debugging mode decides what is mounted

`debug.enabled` reaches `ApiConfig::debug_enabled`, and `router_with` mounts `debug/play-file` and
`debug/play-upload` **only when it is true**. Off, they answer 404 — genuinely absent rather than
carrying a disabled handler — and every `debug.` setting is ignored with them.

That is why the switch takes effect at the next start: the routes are decided when the router is
built. `PUT /api/v1/admin/debug` persists it and says so; `GET /api/v1/debug` reports it and is
public, because a curation tool has to ask before it offers a Play button.

### The lesson from an ACL change that did not survive a restart

The decision this documented is gone, and the finding is not. Setting a route to `admin` over the LAN
made the API report `admin`, and a restart put it back to `public`: a write that mutated the map
inside the API state and stopped there, with **no path from the crate that enforced it to the crate
that stored it**. The thing being lost was a *restriction*, reported as applied.

**What made it invisible: every test drove the API and asserted on the API's own answer, which was
correct at every moment.** Nothing restarted a machine and looked again. That shape — a test that
checks what arrived rather than what it is worth — is the transferable part, and it is why
`Controller::set_session_epoch` and `set_debug_enabled` both persist at once and both deliberately
have **no default implementation**: a default would be a no-op, and a silent no-op is the bug itself.
`a_token_outlives_the_state_that_issued_it` and
`a_token_survives_the_process_that_issued_it` are the tests that would now notice.

## The routes that take a body worth measuring

`POST /debug/play-upload` carries a song rather than a path, for a curator whose machine is in another
room. It was the first upload endpoint here and, for a long time, **the only `DefaultBodyLimit` in the
tree**: axum puts 2 MB on every request, which would refuse every MP3+G pair and every video, so
the layer sits on this route at a gigabyte.

**An UltraStar song crosses both debug routes as its MP3 and a `lyrics` field**, the JSON timeline a
package stores. The package builder reads the `.txt`, because the machine never reads one, and a
`.txt` is not among the extensions an upload may be staged under.

**There are four now**, and the other three are the owner's uploads — `POST /admin/packages/upload`,
`POST /admin/wallpapers` and `POST /admin/audio/soundfonts`. They are the same shape with the
audition-specific parts removed, and the sanitising below is shared rather than copied:
`audition_stem` became `safe_stem` when the second caller arrived. Each carries its own cap and
its own extension list, chosen by kind in one table both the JSON routes and `/admin/`'s forms
read — `km_api::uploads::receive` is the front door for the second of those, so the page cannot
grow multipart handling of its own and come to disagree about what a wallpaper is.

**`km_api::uploads::path_for` is the same front door for a client on the wire**, and it exists
because it did not. `km-admin` read the field name, the cap and the extension list from that module
and wrote the *path* itself — without the `/admin` all three moved behind — so every file it ever
sent reached nothing: a package got the fallback's 404, a picture and a bank got a bare 405 from the
public `GET` twins at those paths. It was invisible because its tests mounted their `wiremock` on the
client's own strings, which is a client agreeing with itself. `km-admin` and `km-package-builder`
both read `path_for` now, and two tests compare it to `routes::SURFACE` — one in this crate, one in
`km-admin`, because the second is the only thing in that program that consults the machine's table.

The package cap is **2 GiB minus one**, and the number is a consequence rather than a judgment:
`DefaultBodyLimit` takes a `usize` and this workspace builds for a 32-bit Android target. Worth
saying out loud, because the obvious change — raising it — does not fail here, it fails on a
platform nobody was thinking about.

### There are two caps per route, and sharing one of them is not sharing the other

**This is the correction to the paragraph above, and it cost the owner's page every upload it
existed for.** "Each carries its own cap and its own extension list, chosen by kind in one table
both the JSON routes and `/admin/`'s forms read" was true of the cap `km_api::uploads::receive`
*validates* against, and it said nothing about the `DefaultBodyLimit` **layer**, which is a
different mechanism on a different router. The JSON routes in `routes.rs` had theirs; the four
routes in `km-admin-pages`' own `Router` had none, so every one of them ran on axum's 2 MB default —
a limit nobody wrote, against payloads of tens or hundreds of megabytes.

Three things about how that hid, each of which will hide the next one:

- **The failure names a parser, not a size.** axum's `Multipart` reads the body itself, so an
  overflow mid-stream is **not** a `413`; it arrives as *"Error parsing `multipart/form-data`
  request"* — about a file that is perfectly well formed. Nothing in the message, and nothing in the
  code, says 2 MB.
- **A small file works.** A 29 KB package installed happily from the same form that stopped an 85 MB
  one at 2,162,688 bytes, so every quick test passes.
- **A test over the response body proves nothing here**, because these forms answer `303` and put
  the outcome in `Location`. Two versions of the regression test passed with the bug present for
  that reason alone.

The rule this leaves: **a router that takes uploads sets its own limits, and nesting does not inherit
them.** `km-admin-pages`' router now names the same `km_api::handlers::MAX_*` constants, so the
transport limit and the validated cap cannot drift apart per kind.

Streamed to disk a chunk at a time through `Field::chunk`, never buffered: a route with a gigabyte
limit that read its body into memory first would be a gigabyte of memory. The staging folder comes
from `Controller::open_audition`, because this crate does the HTTP and the implementation says where —
the same division `play_file` makes about allowed folders, and for the same reason. The owner's
uploads use `open_upload` and `accept_upload` for that division taken one step further: **what
happens after the bytes land is the machine's business too**, and it differs per kind. A package
is installed into the catalog; a SoundFont needs nothing at all, because `soundfont::installed`
reads the directory on every call; a wallpaper has to make the *display* re-resolve which folder
it is watching, which is a piece of machine knowledge this crate should never have held.

**`Controller::play_audition` takes a bare name, not a path**, which is what keeps the containment
rule in one place and makes it trivially correct: a name with no separator cannot escape a join, so
there is nothing to canonicalise and compare. The name is built here from a `stem` **form field** and
an extension allow-listed against `km_kmpkg`'s own constants — never from a part's filename, which is
a client-written header carrying `..`, separators, Windows-illegal characters and an encoding neither
end can rely on. Both halves of an MP3+G pair get the same stem, so `pair_for` hits its fast path.

A `Staged` guard removes the folder on every early return, so a refused stem, an oversized body and a
client that hung up all leave nothing behind. **It covers only the failures before playback starts**;
once the song is playing the folder belongs to the machine, which reclaims it when the song is
displaced — so ownership passes at exactly the moment `keep()` is called, and neither side is ever
holding the other's. Axum's own multipart rejection is plain text with no
code in it, so the handler takes `Result<Multipart, MultipartRejection>` and maps it — every failure
on this surface carries an `error` and a `message`, and a wrong content type should not be the one
exception.

## What a page may not know for itself

Two flags exist so a page can leave a control out rather than offer one that is always refused, and
both are answered by the machine: `PackageDto::removable` and `SoundFontBankDto::removable`. The
product rule is
[`A page asks before it deletes a file; the API does not`](../decisions/api-and-network.md#a-page-asks-before-it-deletes-a-file-the-api-does-not);
what this note records is the shape it takes in the code.

**The trait methods take the row, not an id.** `Catalog::why_not_removable(&InstalledPackage)` and
`package_bytes(&InstalledPackage)` are keyed by the thing the caller already holds, because the
archive's path *is* the whole input — asking by id would make the implementation read the package
list again, once per row, under a lock a page load takes care to stay off. Both are **defaulted**, so
`km-api`'s test double needs to know nothing about `debug.` sections or packages folders.

**They return the sentence, and only a boolean ships.** `not_mine_to_delete` in `karaokemachine` is
one function that both `Catalog::uninstall` and `why_not_removable` call, so the refusal a route
gives and the reason a page prints cannot drift apart. Every one of those sentences names a full
path, so `PackageDto` carries `removable` alone: `/admin/` reads the sentence in-process, a remote
gets the flag. The bank half is `Machine::bank_not_mine_to_delete`, reported on `SoundFontBank`.

**Size is read at the moment of asking.** Not a column on `InstalledPackage`, which is a row out of
an index of songs and has never counted bytes — a stored number goes stale, and a `metadata` call
inside `Catalog::packages` would put one filesystem touch per package on a read path that has none.
The confirmation is a page a person just asked for, so one `metadata` there is free.

### `DELETE /packages/{id}` answered 404 for everything

Found while building the confirmation and fixed in the same change. The handler mapped its error with
`|_| ApiError::not_found(…)`, which is a shape that reads as careful and is the opposite: it collapses
three different failures into the one status that means *it is not here*, and discards the machine's
own sentence explaining which of them happened. A package the machine refuses to delete and a disk
that would not release the file both answered `404 package '<id>'`.

It is now the same `match` `set_package_bank` twelve lines below already used — `NotFound` spelled out
so the id stays in the sentence, everything else through `From<CatalogError>`, which already maps
`Rejected` to 400 and `Failed` to 500. **The lesson is about the idiom, not this route**: a
`map_err(|_| …)` on a typed error is a decision to throw information away, and it should have to be
argued for at the call site.

## Discovery and events

`_karaokemachine._tcp.local` over mDNS/DNS-SD with `mdns-sd` (pure Rust, works on desktop and
Android), with TXT records carrying the instance id, name and API prefix -- and deliberately nothing
about the password, which `GET /api/v1/discover` answers to a client that asks rather than broadcasting
to a segment that did not.
**Because mDNS is blocked on plenty of networks**, the always-public `GET /api/v1/discover` returns
the same payload, and the display shows the URL and a QR code.

**The browse side is a `watch::Watcher` held open rather than a `browse(timeout)` per question.**
One daemon and one thread for the life of the process, folding announcements into a `Registry` that
answers instantly — and a `ServiceRemoved` marks a row *absent* rather than removing it, because on
Android silence means the multicast lock was released and not that anything moved. The registry is a
type apart from the daemon so that every test drives one and nothing opens a socket. Two clocks:
`Instant` in the registry, `SystemTime` in `known::Known`, which is the record a client writes down —
id, address, name and when the machine last answered. Nothing on the advertising side changed.

WebSocket events: `state`, `queue_changed`, `song_started`, `song_ended`, `lyric_line`,
`settings_changed`, `mics_changed`, `wallpaper_changed` and `desync`. **Playback position rides on
`state` at about 4 Hz — per-syllable position is never streamed**; the display reads it from the
shared atomic and remotes interpolate locally from the lyric timeline.

### The machine's own log

A second WebSocket, `GET /api/v1/admin/logs/stream`, carrying `record` and `lagged` frames, with
`GET /api/v1/admin/logs` answering the same records as a tail. The decision is
[`The machine's own log is a route, and it is the owner's`](../decisions/api-and-network.md#the-machines-own-log-is-a-route-and-it-is-the-owners);
what follows is how it is wired.

**`km-logtap` holds the ring and `km-api` holds no channel of its own**, which is the difference from
`events`: that module owns the `Events` channel as well as the wire type, where here the buffer
belongs to a platform crate and only the wire half is in `dto.rs`. `ApiState` keeps a `OnceLock<LogTap>`
beside the one holding `Power`, read by `router_with` while the router is being built — so a tap has
to be installed before serving, exactly as a power capability does. A concrete type rather than a
trait: what powering a box off *does* varies per host, where a ring of records behaves the same
everywhere and has one implementation.

**`api_router` takes a `Capabilities`, not a pair of bools.** Power and logs are read from the same
state and mounted differently — the dev mirror gets the log routes and not the power ones — so the
call sites name their two fields rather than leaving a reader to count arguments.

**`LogTap::tail_and_subscribe` takes both under the ring's lock**, which `push` also holds while it
appends. A record therefore lands in the tail, on the reader, or in both where a push had appended
and not yet sent; `pump_logs` drops the overlap by `seq`. The test that protects it uses two real
threads and a barrier, because a spawned task on the single-threaded runtime a `#[tokio::test]` gets
never runs beside the test body and so passes whichever way round the snapshot is taken.

**Nothing in `pump_logs` or its send helper emits a `tracing` event**, which is why it does not reuse
`send_event` — that one reports a serialization failure at `error!`. A line said while feeding a
reader is taken by the tap and sent straight back to it.


## Refusal codes

`Refusal { code: Option<&'static str>, message: String }` rides inside `ControlError::Unavailable`
and `CatalogError::Unavailable`. `ApiError::unavailable` puts the code in the body's `error` field,
falling back to `unavailable`, so a client that knows none of the finer codes gets the generic
refusal rather than nothing.

| Code | Raised by | What a surface says |
|---|---|---|
| `no_key_midi` / `no_key_video` / `no_key_cdg` | a key change on a song with no key | `error-no-key`, selecting on the kind |
| `no_tempo_midi` / `no_tempo_video` / `no_tempo_cdg` | a tempo change on a song with no tempo | `error-no-tempo` |
| `no_melody_midi` / `no_melody_video` / `no_melody_cdg` | a melody toggle on a song that has none | `error-no-melody` |
| `no_melody_channel` | a MIDI song where detection abstained | `error-no-melody-channel` |
| `nothing_playing` | pause with nothing playing, and skip with nothing playing and demo mode off | `error-nothing-playing` |
| `nothing_loaded` | seek or restart with nothing loaded | `error-nothing-loaded` |
| `nothing_queued` | play with an empty machine and an empty queue | `error-nothing-queued` |
| `no_sound` | the engine cannot play at all | `error-no-sound` |
| `queue_full` | the queue is at `MAX_QUEUED` | `error-queue-full` |
| `unavailable` | everything else, including anything owner-only | `error-unavailable` |

**The kind is in the code rather than beside it** because a sentence the machine composed cannot be
taken apart again — `article_name()` bakes English's article agreement in, and Portuguese needs its
own. `km-remote-pages` strips the suffix to find the message family and reads it again to pick the
Fluent variant.

**The two vocabularies are not one dependency.** The machine links the pages, so the codes are spelled
on both sides; `every_refusal_this_machine_sends_is_a_sentence_the_remote_has`, in `karaokemachine`, is
the one place both spellings can be seen at once and is what stops them drifting.
