# Research: a fair queue that an operator can correct

**Findings, and no decision.** A guest's song goes straight into the queue, and
[`Four access levels, and the method and path decide them`](../decisions/api-and-network.md#four-access-levels-and-the-method-and-path-decide-them)
is what lets it. A fair queue would add a level to that entry, change its default and add a surface.

Read on 2026-10-11, from this repository and from the OpenKJ source at commit `47d54f44674f`.
Auto-KJ's public site was read on 2026-10-12.
**Nothing here was built or run.** Every claim about this machine is a reading of code or of a
decision, and the marker on each says how far to trust it.

**Summary.** A guest's song becomes a *request*, and a rule moves it into the queue. The rule keeps
one song for each singer in the queue, so the singers take turns. The rule needs nobody in charge.

A singer is a *seat* that the machine issues, and the typed name is its label. A phone holds a token
for each of its singers. An operator creates a singer with no phone, and adds songs for any singer.

An operator corrects what the rule did, and that is the exception. The operator declines a request,
accepts one early, or moves an entry. The queue stays the flat list it is.

A fifth access level, `request`, carries the whole design. A new machine starts its room at that
level, so the fair queue is what a machine does with no setup.

The design needs two new lists, a few routes, events on the existing stream and one page.

Every queued song also shows a rough wait, on the remote and on the machine's screen. Section 12
shows how a public venue runs a night on the design.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository's code. High confidence. |
| **[decision]** | Read from an entry in `docs/decisions/`. High confidence. |
| **[openkj]** | Read from the OpenKJ C++ source. High confidence for a structure, medium for a behaviour. |
| **[autokj]** | Read from Auto-KJ's public site and the text of its simulator. Medium confidence, because no source was read. |
| **[chosen]** | A preference the owner stated for this design. No decision records it. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. The question

A user asked for fairness between singers, or for an operator who reorders, approves or limits what
is queued. Four answers exist.

| Answer | What it does | Size |
|---|---|---|
| A limit per singer | Refuses a song when the singer already has some waiting | Small |
| A rule for placing a song | Puts a singer's second song after everybody's first | Medium |
| A rotation of singers | Replaces the list of songs with a list of singers | Large |
| Operator mode | Holds a guest's song as a request until an operator queues it | Medium |

**[inferred]** One design gives the first three and keeps the fourth for the exception. The limit
holds a song as a request where it would refuse it. The rule that releases a held song is the
placing rule. The two together give the rotation on a flat list.

**[inferred]** The same design serves a night where only an operator enters songs. The operator
types them in any order, and the rule still gives the singers their turns.

## 2. The queue on this machine

**[repo]** `km_queue::Queue` is a flat list of songs with a cap of 200, `MAX_QUEUED`. A
`QueueEntry` holds an opaque `id`, the song `number`, a `title`, an `artist` and an optional
`singer`.

**[repo]** `Queue::add` takes a `QueueRequest`, so the crate already uses the word *request* for
another thing.

**[repo]** `Queue::pop` takes the first entry out when its song starts. The playing song is
therefore not in the queue.

**[repo]** The `singer` is free text. The singer's remote keeps it in the "singing as" box on its
Setup tab and sends it with each song.

**[decision]** The singer name is a label and never an identity, and the machine keeps no accounts.
The queue holds no owner for an entry, which is why moving or removing one needs the control level.

**[repo]** `routes::required_access` reads the method and the path and nothing else. It gives
`POST /queue` and `PUT /settings` the `queue` level, and gives every other write the `control`
level. A route added later is at the `control` level until the `match` names it.

**[repo]** Every `GET` is at the `view` level, and so is the event stream.

**[decision]** The room level is what a phone with no code gets, and it starts at `queue`. An owner
may set it to `view` or `control`. A queue code and a control code each raise one phone.

**[repo]** `ops::enqueue` is the one implementation of adding a song. It resolves the number, adds
the entry and publishes `Event::QueueChanged` with the whole queue. `POST /queue` answers 201 with
the entry's id and position.

**[repo]** A phone at the control level already reorders by hand. The Queue tab folds `↑ ↓ ✕` behind
a ⋯, and the Songs tab folds `↑ next` and `▶ now` behind another.

**[decision]** The queue lives in memory, and ending the process loses it.

**[decision]** A token carries its level, and `POST /api/v1/login` gives one for a code or for the
admin password. A phone at the room level holds none.

**[repo]** The singer's remote is one set of pages, `km-remote-pages`, which a server renders. The
machine serves them, and so does the offline remote. The Android and iOS remotes show them in a
WebView, from a copy inside the app.

**[repo]** The machine's own pages call `km_api::ops` in the same process. Every other host reaches
the machine over HTTP through `MachineClient`.

**[repo]** The pages keep the singer name in a cookie, `km_singer`, and send it as `singer` in the
body of `POST /queue`. That body refuses a field it does not know.

**[repo]** The pages never show the machine's own words for an error. They pick a fixed sentence
from the status and the error code, and they log the machine's message.

| The machine answers | The pages say |
|---|---|
| 401 or 403 | "This needs a code. Type one on the Setup tab." |
| 409 with a code the pages do not know | "The machine cannot do that right now." |
| Any other 4xx | "That did not work. Try again." |

**[repo]** The pages check the phone's level before they send a write. A phone below the level gets
the first sentence, and the machine hears nothing.

**[repo]** A remote and a machine compare no versions.

**[repo]** A token is `v2.<level>.<expires>.<nonce>.<mac>`, and the parser refuses any other shape.
The stored admin password hash is the signing key, so a token outlives a restart.

**[decision]** Each song starts at its own key and tempo, and
[`A key and a tempo belong to one song`](../decisions/audio.md#a-key-and-a-tempo-belong-to-one-song)
is the entry. `playback.transpose` and `playback.tempo_ratio` are the owner's defaults, and a change
from a remote never writes them.

**[repo]** A `QueueEntry` carries no key, no tempo and no duration. Key and tempo are settings of
the machine, which `PUT /settings` changes. A song's `duration_ms` is in the catalog's answer for that song.

**[repo]** `ops::enqueue` reads that answer before it adds the entry. `NowPlayingDto` carries the
playing song's `duration_ms`, and the playback snapshot carries its `position_ms`.

**[repo]** The machine's screen draws the queue from the `QueueEntry` list itself. It has an overlay
that lists every waiting song, and a line that names the next one.

## 3. What OpenKJ does

OpenKJ is a desktop program for a paid host at a venue, under the GPL-3.0 licence. The source read
is `https://github.com/pkircher29/openkj`, a fork of `OpenKJ/OpenKJ`.

**[openkj]** Its rotation is a list of *singers*, and each singer owns a queue of songs. The show
plays one song for each singer in turn. A singer with ten songs waiting still sings once in a round.

**[openkj]** `TableModelRotation::singerAdd` places a new singer at one of three positions, and the
host picks one each time.

| Position | Where the singer lands |
|---|---|
| Fair | Before the current singer, which is one full round away |
| Bottom | The end of the list |
| Next | After the current singer |

**[openkj]** A guest's request holds `singer`, `artist`, `title`, `key` and a time. It reaches a
requests dialog and never the rotation. The host assigns it to an existing singer or to a new one.

**[openkj]** The host has an *accepting* switch that opens and closes requests. The host drags
singers to reorder them, and drags songs inside one singer's queue.

**[openkj]** Each singer's row shows a wait in turns and in minutes. The model adds the durations of
the songs ahead and the time left in the current one.

**[openkj]** A singer's identity is the typed name. The model also saves regular singers with their
songs between shows, and marks a singer who has not sung tonight.

**[openkj]** The rotation code holds no limit per singer and no cooldown. The README names an
automated request mode, and its source was not read.

## 4. What Auto-KJ does

Auto-KJ is a closed desktop program for a host, with a page for each singer's phone. Its site calls
its rotation rules a *fairness engine*, and `https://auto-kj.com/#simulator` runs them in a page.

**[autokj]** A singer holds one seat in the line, however many songs the singer asks for. The extra
songs wait in that singer's *pocket* and come out one in each round.

**[autokj]** A singer who arrives during the show takes the last seat. The site's rule for a late
arrival is the first place that pushes nobody back.

**[autokj]** The site gives three properties that a singer can check for themselves.

| Property | Meaning |
|---|---|
| Stable order | A singer's place never gets worse because of what another singer did |
| Arrivals do not cut | A later arrival sings after an earlier singer, in every round |
| One price for a song | One song costs one turn, whoever sings it |

**[autokj]** A song that is in the line, or that somebody sang tonight, is marked *taken* on every
phone. The host picks the whole night, a window of 60 or 90 minutes, or no check.

**[autokj]** A singer who is absent at their turn moves back and gets a strike. A second absence in
a row removes the singer.

**[autokj]** A singer can pause, and a paused singer keeps moving up the line. The engine sets the
singer aside just before their turn, and a return puts them close to the front.

**[autokj]** A duet costs each of its singers a turn when both are in the line. A setting lowers
that cost for the singer who does not lead.

**[autokj]** The program proposes a merge when two names are close, such as `Dave` and `David`. One
action joins the two pockets into one seat.

**[autokj]** Each phone shows its singer's place and a wait in minutes. The wait moves as songs run
long or short.

**[autokj]** The host drags any seat to any place, and turns any rule off in the settings.

**[inferred]** The seat and the pocket are the rule of section 7 under other names. The queue entry
is the seat, and the singer's waiting requests are the pocket.

## 5. A fifth level carries the design

**[inferred]** A setting that changed what the `queue` level means would break the decision.
`required_access` would have to read that setting beside the method and the path.

**[inferred]** A level between `view` and `queue` avoids that. Each level includes the ones below
it, as the four do.

| Level | What it adds |
|---|---|
| `view` | Every `GET` |
| `request` | Registering a singer, `POST /requests`, and withdrawing a request |
| `queue` | `POST /queue` for a song with no singer, and `PUT /settings` |
| `control` | Every other write outside `/admin/`, which includes acting for any singer |
| `admin` | Everything under `/api/v1/admin/` |

**[chosen]** The room level starts at `request`. A new machine gives singers their turns with no
setup, and an owner who wants the direct queue raises the room to `queue`.

**[chosen]** An update moves a machine to `request` when its owner never set the room level. A level
the owner chose stays as chosen.

**[repo]** `settings.api.room_access` is a plain level whose default is `queue`. A stored `queue` is
therefore the default and a choice at once. The update can keep only `view` and `control`.

**[chosen]** The update moves a stored `queue` to `request`. An owner who wants the direct queue
chooses it again once.

**[chosen]** A phone with the queue code asks for a song for its singer, as any phone does. Adding a
generic song sits behind the ⋯, beside what the control level folds there.

**[chosen]** An installed remote of an earlier build gets no handling of its own. Few phones hold
one, and the remedy is to update the remote.

**[inferred]** The machine serves the web remote, so that remote always matches the machine. Only an
installed remote can be behind.

**[repo]** An installed remote cannot say that it is too old. It holds its own copy of the pages,
and those pages show a fixed sentence for every refusal.

**[inferred]** In a room at `request`, an installed remote of an earlier build sends nothing. Its own
check finds the phone below `queue`, and the guest reads "This needs a code".

**[inferred]** A night where only the operator enters songs is a room at `view`. The guests see the
queue and their place, and cannot ask for a song. That night needs no setting of its own.

**[chosen]** `POST /queue` adds a song with no singer and takes no name. A named song always goes
through the rule. Only the `control` level puts a singer's second song in early.

**[inferred]** `POST /queue` should refuse a body that names a singer. A client that sends a typed
name then fails with an error, where a dropped name would fail without one.

**[inferred]** A phone at `request` cannot reach `PUT /settings`. A guest therefore cannot change the
key of a song that another singer is singing. Section 7 gives each song a key of its own.

**[repo]** Each route at the `request` level needs a line in `required_access`. A route for the
operator needs no line, because a new write defaults to `control`.

**[decision]** The decision's title counts four levels, its room level takes three values, and its
room starts at `queue`. All three would change.

## 6. A singer is a seat

**[chosen]** Every song a guest asks for belongs to a registered singer. A song with no singer is
*generic*, and the machine has no third kind.

**[inferred]** A seat holds an id and a name. The machine issues it, and a random token proves that
a caller holds it. The seat is no access level, so `required_access` does not read it.

**[inferred]** The seat's token needs no signature. The machine keeps the seats in memory and
compares a caller's token with the one it stored. A level's token has no part for a seat's id.

**[inferred]** A stored token also ends with the evening, as a seat must. A signed one would outlive
a restart, as a level's token does.

**[inferred]** The pages keep a device's seats where they keep the singer name, in a cookie. On
Android and iOS that cookie lives in the WebView.

**[inferred]** Each seat action belongs in `km_api::ops`, because the machine's own pages call it
there. The pages' `Machine` trait then gains the same actions for its three implementations.

**[inferred]** The name is what people see, and the token is what proves. Without the token anybody
could type a held name and withdraw that singer's songs.

**[chosen]** A phone registers a singer on its first request. The remote registers it when a guest
first picks a song, and keeps the token. It sends the name in the "singing as" box, or none. No join step comes before the catalog.

**[chosen]** A name is optional on a new machine, and the machine works in full with every singer
unnamed. A setting makes the name required, for an owner who wants names on the screen.

**[inferred]** A seat therefore needs no name. The remote registers an unnamed seat for its device
and keeps the token. The rule gives that seat its turns as it gives any other.

**[inferred]** An unnamed request must not become a generic song. A generic song passes the rule,
so every guest would drop the name to get ahead.

**[inferred]** The operator's page still needs to tell unnamed seats apart. The machine gives each a
number, such as `Guest 4`, which is a label and no name.

**[chosen]** The television shows no singer for an unnamed seat's song, as it shows none for a song
with no singer today. The number appears on the operator's page and on the guest's own phone.

**[chosen]** One device holds one unnamed singer at most, and every further singer on it gives a
name. People who share a tablet can then tell their songs apart.

**[chosen]** A guest who types a name later names the same seat. The seat keeps its place, its
waiting songs and its token, and the name must be free as for any rename.

**[inferred]** An unnamed seat has no name to reclaim. A phone that loses its token registers a new
seat, and the old one ends with its last song.

**[chosen]** One device holds many singers, because a home passes one phone or one tablet around. A
setting caps the singers on one device, and it starts with no limit.

**[chosen]** The remote still shows one "singing as" box, as a phone with one singer needs. Adding
another singer is a separate and smaller control, and the remote never presents it as the usual
way.

**[inferred]** A device with one singer therefore never shows a choice of singers. The choice
appears with the second singer, and each request then names one of them.

**[inferred]** The machine counts a device's singers by an id that the remote makes and sends. A
remote that sends a new id escapes the cap. The cap therefore stops a mistake and not a cheat.

**[chosen]** A name is unique among the evening's singers. Registering or renaming to a held name is
refused. The comparison ignores case, surrounding spaces and accents.

**[inferred]** A unique name also makes the television and the operator's page unambiguous, because
both show names and nothing else.

**[chosen]** A lost token has one way back that needs nobody. Registering the name of a seat that
has nothing waiting and nothing queued returns that seat with a new token. The old token ends.

**[chosen]** The same registration is refused while the seat holds a song. A caller at the `control`
level then reissues the seat. A reclaim by name alone would hand one singer's songs to another.

**[chosen]** An operator creates a singer with no phone and no token. An operator also adds a song
for any singer, whoever registered it.

| Route | Level | Proof |
|---|---|---|
| `POST /requests` | `request` | The singer's token |
| `POST /singers/{id}/requests` | `control` | None, because the level is enough |

**[inferred]** The reissue also hands an operator's singer to a phone, when that singer picks one
up later.

**[chosen]** A seat lives on one device. A second device is refused the name, and the reissue is the
way to move a singer from one device to another.

**[chosen]** A seat ends with the evening. The machine stores no list of people, and a name is free
again the next night.

**[inferred]** The seat keeps the decision that the machine has no accounts. It holds no password
and nothing that outlives the evening. It does let the machine tell two phones apart.

**[inferred]** A guest who clears the browser's storage and types a new name gets a second seat.
Only an account stops that. Section 9 gives the operator a merge for it.

## 7. The rule that accepts a request

**[inferred]** The rule is one sentence: the queue holds one song for each singer. Everything below
follows from it.

- The machine accepts a request at once when its singer has no entry in the queue.
- Any other request waits in the request list.
- When a singer's entry leaves the queue, the machine accepts that singer's oldest waiting request.

**[chosen]** The rule treats an operator's request as it treats a phone's. An operator who types in
a pile of slips in any order still gets a queue of fair turns.

**[chosen]** A singer's next request goes in when that singer's song starts. The singer sees the
next place while singing, and the queue is never empty while requests wait.

**[inferred]** An entry also leaves the queue when the operator removes it or the guest withdraws
it. The rule treats all three alike.

**[chosen]** An accepted song goes to the end of the queue. No entry moves back for it, which is
Auto-KJ's stable order.

**[inferred]** While the rule alone fills the queue, the queue is one round. Three singers with three
requests each then sing in turn, as in OpenKJ's rotation. A new singer waits one round at most.

**[inferred]** An operator who accepts a song early makes a second round. A new singer's first song
can then follow another singer's second. That is the cost of the stable order.

**[inferred]** The rule places a song once and never moves an entry afterwards. Whatever an operator
moves therefore stays where the operator put it.

**[inferred]** The rule needs no record of the evening. It reads the queue and the request list, and
both exist the moment the machine starts.

**[inferred]** A generic song counts as a singer of its own. It goes straight to the end of the
queue, and it holds back no other song.

**[chosen]** The rule always runs, and no stricter form holds every request for approval. An operator
who wants every song in hand sets the room to `view` and enters each one.

**[inferred]** The request list needs two caps. One covers the whole list, because a phone at the
room level could otherwise fill memory. One covers each singer. A refusal says which cap it met.

**[chosen]** The whole list holds 1,000 requests, a constant as the queue's 200 is. A setting caps
one singer's waiting requests, and it starts at 10.

**[inferred]** A full queue accepts nothing, and its requests wait until an entry leaves.

**[chosen]** A song carries its own key and tempo. The singer picks them with the request, the queue
entry keeps them, and the machine applies them when the song starts.

**[chosen]** The remote does not show key and tempo when a guest asks for a song. Few singers set
them, so the two controls sit behind a fold that a regular opens.

**[decision]** A song with no key of its own starts at the owner's defaults. No singer inherits what
the singer before chose.

**[decision]** Only key and tempo start again with each song. Melody, music volume and lyric offset
belong to the room and its equipment, and they stay as set.

**[chosen]** The singer of the playing song changes its key and tempo. The singer's token is the
proof, and it opens that singer's own song and no other.

**[inferred]** That change needs a route of its own at the `request` level. `PUT /settings` stays at
`queue`, and a generic song's key stays with a phone that holds a code.

**[repo]** A `QueueEntry` has neither field today, so the key on each entry is a feature of its own.
The fair queue can be built before it, and a request then carries no key.

## 8. What a guest sees

**[inferred]** A guest who gets no answer asks again or walks to the operator. So the guest's phone
must show each request from the asking to the singing.

**[inferred]** A request keeps the id of its queue entry once the machine accepts it. One record
then carries the whole course of a song.

| State | Meaning |
|---|---|
| Waiting | The singer already has a song in the queue |
| Queued | Accepted, with the entry's position |
| Played | The song started |
| Declined | An operator refused it |
| Withdrawn | The guest took it back |

**[inferred]** The list of requests is a `GET`, so the whole room reads it. A phone picks out the
requests of its own singers by their ids.

**[inferred]** The singer's token is the proof for withdrawing a request. The same token withdraws
the song after the machine accepts it. A guest who queued the wrong song needs no control code.

**[inferred]** A finished request must leave the list, or the played and declined ones fill the cap.
A fixed time after it finishes gives the guest's phone long enough to read it.

## 9. What an operator does

**[inferred]** An operator acts for a singer who has no phone, or where the rule gave the wrong
answer.

| Action | When it is needed |
|---|---|
| Create a singer, and add a song for any singer | A guest has no phone, or the room is at `view` |
| Decline a waiting request | The song is unwelcome |
| Accept a waiting request early | A singer gets a second song in the queue, next or at the end |
| Move or remove a queue entry | The order needs a change, which `POST /queue/{id}/move` does today |
| Merge two singers | One person registered two names |
| Reissue a seat | A singer lost a token while holding a song, or picked up a phone |
| Remove a singer | The singer went home, and the seat's queued and waiting songs must go |
| Stop and start accepting | The night is ending, and the machine must refuse a new request |

**[chosen]** With accepting off, the machine refuses a new request and the phone says the night is
closed. A request that already waits still enters the queue in its turn.

**[chosen]** Removing a singer is the answer to one who left early. With nobody operating, that
singer's songs play until somebody skips each one.

**[chosen]** A guest who asks for a song that is queued, or that somebody sang tonight, gets a
warning. The guest then confirms or picks another, and the machine refuses nothing.

**[inferred]** Adding a song is the action an operator repeats all night. Singer, song and the add
belong on one screen. Typing a name finds a singer first, and offers a new one when none matches.

**[inferred]** A merge moves one singer's requests and entries to the other. The rule then finds two
entries for one singer, leaves both, and accepts no third.

**[decision]** `/admin/` is the owner's page and is a third surface on purpose. A control code is
not the admin password, so the operator's page belongs outside `/admin/`.

**[repo]** The singer's remote already hides each button below the phone's level, through a class on
`<body>`. An operator's tab in that remote is therefore cheaper than a page of its own.

**[inferred]** The tab wants two columns on a wide screen: requests on one side and the queue on the
other. A tablet or a laptop suits it better than a phone.

**[decision]** The offline remote reaches the machine over HTTP. So every operator action must be an
API route, whichever page draws it.

**[inferred]** The machine's keypad cannot pick a singer, so a number typed there adds a generic
song. Whoever stands at the machine already has every control it offers.

The helpers worth having beyond those actions, and what each one needs:

| Helper | What it needs |
|---|---|
| A panel of singers | A count of songs sung tonight for each singer, which nothing keeps today |
| A warning on a request | The queue, and the list of songs sung tonight |
| A proposed merge | A measure of how close two names are, or two seats on one device |
| A song marked taken | `Queue::contains_number`, and the list of songs sung tonight |

## 10. The evening

**[inferred]** The panel of singers and the warning both need a record of the evening. The rule does
not, so the record is no part of the smallest build.

**[inferred]** The evening starts when the process starts, because the queue starts then too. A
request and a seat therefore do not outlive a restart.

**[inferred]** One action at the `control` level starts a new evening without a restart. It clears
the request list and the record, and it ends every seat.

## 11. The wait

**[chosen]** Each queued song shows a rough time until it starts. The remote shows it, and so does
the queue overlay on the machine's screen. No setting turns it off.

**[chosen]** The figure tells a singer whether to stay near the microphone, and a rough one does
that. It has two forms.

| The wait | What the entry shows |
|---|---|
| A minute or more | *about N min*, rounded to the minute |
| Less than a minute | *less than a minute* |

**[inferred]** The wait has two parts. One is the time left in the playing song. The other is the
sum of the durations of the entries ahead.

**[inferred]** `QueueEntry` gains `duration_ms`, because the machine's screen and the API both read
that type. The screen adds the durations itself, and `ops::enqueue` already holds the figure.

**[inferred]** For a remote, the machine sends the second part and the client adds the first.
`QueueEntryDto` gains `duration_ms` and `ahead_ms`, and `ahead_ms` is the sum for the entries
before it.

**[inferred]** The split keeps the figure moving between events. `Event::QueueChanged` fires when
the queue changes, and the time left in the playing song changes every second. A client already
reads the position, so it counts the wait down without a new event.

**[inferred]** The figure ignores the time between two songs, a pause and a changed tempo. Each is
small beside a figure rounded to the minute.

**[inferred]** The stable order of section 7 keeps the figure honest. No accepted song moves an
entry back, so a wait grows only when an operator moves an entry.

**[inferred]** A request in the *Queued* state shows the wait of its entry. A request in the
*Waiting* state has no place yet, and it shows no figure.

**[chosen]** The machine's screen shows how many requests wait. The queue holds one song for each
singer, so the count tells the room how much singing stands behind it. The count is the one thing
about requests that the screen shows.

**[inferred]** The wait needs no seat and no rule. It can be built before the fair queue, on the
queue the machine has.

## 12. A public venue

**[inferred]** The design runs a night at a bar or a hall. The host sets the machine up once, and no
piece needs a second program.

| Setting | Value at a venue | Reason |
|---|---|---|
| The room level | `request`, or `view` where the host enters every song | `request` lets each table ask from a phone |
| The control code | Set, and held by the host | It declines, accepts early, reorders and acts for a singer |
| The queue code | Not handed out | It opens `PUT /settings` and the generic song, which passes the rule |
| A required name | On | The host calls each singer by the name on the screen |
| The cap on one singer's requests | Low | One table cannot fill the request list |
| The admin password | Set, and held by the owner | A host for one night needs no settings |

**[inferred]** A night then runs in four steps.

1. A guest opens the remote on a phone, types a name, and asks for a song.
2. The rule puts the song in the queue, or holds it until that singer's turn comes round.
3. The guest's phone shows the song's place and its wait.
4. The machine's screen shows the queue with a wait on each song, so the room knows who is near.

**[inferred]** The night needs no host at the desk. A host who leaves loses nothing but the
corrections of section 9.

**[inferred]** No guest's phone changes the sound of another singer's song. Section 5 keeps
`PUT /settings` above the `request` level.

**[inferred]** No code raises a phone to `request` alone. A venue that wants only its own guests to
ask therefore relies on its network, or sets the room to `view`.

**[inferred]** Every phone must reach the machine over the venue's network. A guest network that
keeps its clients apart from each other also keeps them from the machine.

### What the change that builds the design must write

**[repo]** `CLAUDE.md` calls the project a machine like a commercial home unit, and no decision
names a venue. The manual says what the machine does, and the site publishes the same. So none of
the four below can land before the design does.

| Where | What to write |
|---|---|
| `docs/decisions/` | An entry that a hosted public night is served, beside the home the machine is built for |
| `docs/decisions/api-and-network.md` | The access levels entry gains the fifth level and the new room default |
| `docs/manual/` | A chapter for a venue: the table above, the four steps, and the two network limits |
| `site/index.html` and each language's page | One short section that says a host can run a public night |

**[repo]** `tools/dist/site.sh` refuses two language pages whose sections differ. The site section
therefore lands in every language at once.

## 13. What was not examined

- Whether the queue overlay has the width for a wait beside each title.
- Where a count of requests fits on the machine's screen.
- Whether a venue needs anything for the public performance of its songs.
- OpenKJ's automated request mode, and the hosted request service its clients use.
- A duet, which needs two singers on one entry.
- A singer who is absent or who pauses, which needs the machine to know who is in the room.
- How the demo and an empty queue behave while requests wait behind a closed accepting switch.
