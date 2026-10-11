# Research: a queue an operator fills from requests

**Findings, and no decision.** A guest's song goes straight into the queue, and
[`Four access levels, and the method and path decide them`](../decisions/api-and-network.md#four-access-levels-and-the-method-and-path-decide-them)
is what lets it. An operator mode would change that entry and add a surface.

Read on 2026-10-11, from this repository and from the OpenKJ source at commit `47d54f44674f`.
**Nothing here was built or run.** Every claim about this machine is a reading of code or of a
decision, and the marker on each says how far to trust it.

**Summary.** In operator mode a guest's song becomes a *request*, and an operator moves it into the
queue. The operator decides the order, so the queue stays the flat list it is. The access levels
already separate the guest from the operator, so the mode needs no new credential.

The mode needs one new list, a few routes, events on the existing stream and one page. It also needs
an answer to four product questions, and section 7 lists them.

The mode serves a night with a host. It does nothing for a room with nobody in charge, and a limit
per singer answers that case separately.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository's code. High confidence. |
| **[decision]** | Read from an entry in `docs/decisions/`. High confidence. |
| **[openkj]** | Read from the OpenKJ C++ source. High confidence for a structure, medium for a behaviour. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. The question

A user asked for fairness between singers, or for an operator who reorders, approves or limits what
is queued. Four answers exist, and this note examines the last.

| Answer | What it does | Size |
|---|---|---|
| A limit per singer | Refuses a song when the singer already has some waiting | Small |
| A rule for placing a song | Puts a singer's second song after everybody's first | Medium |
| A rotation of singers | Replaces the list of songs with a list of singers | Large |
| Operator mode | Holds a guest's song as a request until an operator queues it | Medium |

## 2. The queue on this machine

**[repo]** `km_queue::Queue` is a flat list of songs with a cap of 200, `MAX_QUEUED`. A
`QueueEntry` holds an opaque `id`, the song `number`, a `title`, an `artist` and an optional
`singer`.

**[repo]** The `singer` is free text. The singer's remote keeps it in the "singing as" box on its
Setup tab and sends it with each song.

**[decision]** The singer name is a label and never an identity, and the machine keeps no accounts.
The queue holds no owner for an entry, which is why moving or removing one needs the control level.

**[repo]** `routes::required_access` gives `POST /queue` the `queue` level and gives every other
write the `control` level. A route added later is at the `control` level until the `match` names it.

**[decision]** The room level is what a phone with no code gets, and it starts at `queue`. A queue
code and a control code each raise one phone.

**[repo]** `ops::enqueue` is the one implementation of adding a song. It resolves the number, adds
the entry and publishes `Event::QueueChanged` with the whole queue. `POST /queue` answers 201 with
the entry's id and position.

**[repo]** A phone at the control level already reorders by hand. The Queue tab folds `↑ ↓ ✕` behind
a ⋯, and the Songs tab folds `↑ next` and `▶ now` behind another.

**[decision]** The queue lives in memory, and ending the process loses it.

**[repo]** `QueueEntryDto` carries no duration. A song's `duration_ms` is in the catalog's answer
for that song.

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

## 4. The mode

**[inferred]** One setting turns the mode on, and it is off on a new machine. With it on, the two
lower levels change meaning and nothing else does.

| Level | Mode off | Mode on |
|---|---|---|
| `queue` | Adds a song to the queue | Adds a song to the request list |
| `control` | Adds, moves and removes any entry | The same, and accepts or declines a request |

**[inferred]** A request holds the song, the singer name, the key the guest chose and the time
asked. It has three states: waiting, accepted and declined.

**[inferred]** The list needs a cap as the queue has one. Without it a phone at the room level fills
memory, because nothing drains the list while no operator is present.

**[inferred]** Two shapes fit the routes. `POST /queue` could answer 202 with a request where it
answers 201 with an entry. Or a `/requests` resource could stand beside `/queue`.

**[inferred]** The second shape is the safer one. A client built before the mode reads a 201 and an
`entry_id`, and a 202 with another body breaks it without an error.

**[repo]** A `POST /requests` needs a line in `required_access` to reach the `queue` level. Accept
and decline need no line, because they default to `control`.

**[inferred]** The stream needs a requests event beside `QueueChanged`. The operator's page reads
it, and so does a guest's phone that is waiting for an answer.

## 5. What a guest sees

**[inferred]** A guest who gets no answer asks again or walks to the operator. So the guest's phone
must show each request and its state.

**[repo]** A phone at the room level holds no token. The machine cannot tell two such phones apart.

**[inferred]** The machine does not need to. The phone can keep the ids of its own requests and read
each one's state. That keeps the decision that a singer name is a label.

**[inferred]** A guest who withdraws a request must prove it is theirs. An id that cannot be guessed
is that proof.

**[repo]** The queue's ids count up from zero, so a request id needs another source.

**[inferred]** The operator still groups requests by the typed name, as OpenKJ does. Two spellings
of one name are two singers until the operator merges them.

## 6. The operator's page

**[inferred]** The page wants a wide screen: requests on one side and the queue on the other. A
tablet or a laptop suits it, and a phone does not.

**[decision]** `/admin/` is the owner's page and is a third surface on purpose. A control code is
not the admin password, so the operator's page belongs outside `/admin/`.

**[repo]** The singer's remote already hides each button below the phone's level, through a class on
`<body>`. An operator's tab in that remote is therefore cheaper than a page of its own.

**[decision]** The offline remote reaches the machine over HTTP. So every operator action must be an
API route, whichever page draws it.

The helpers worth having, and what each one needs:

| Helper | What it needs |
|---|---|
| Accept to fair, next or bottom | A position computed over the flat list |
| A panel of singers | A count of songs sung tonight for each name, which nothing keeps today |
| A warning on a request | The queue, and the list of songs sung tonight |
| Drag to reorder | Nothing new; `POST /queue/{id}/move` does it |
| A wait on each row | A duration on each queue entry |
| An accepting switch | One more setting |
| Merging two singers | A rename across the request list and the queue |

**[inferred]** *Fair* has a meaning on a flat list. A round is a run of entries in which no singer
appears twice. The song goes at the end of the first round that does not hold its singer.

**[inferred]** The panel of singers and the warning both need a record of the evening. That record
needs a moment at which the evening starts again, and the machine has none.

## 7. Open questions

1. **What does the machine's own keypad do in this mode?** A number typed at the machine queues the
   song. It could stay direct, because whoever stands at the machine is the operator. It could also
   become a request.
2. **What happens when nobody operates?** The queue empties while requests wait. The television
   could show a count of waiting requests. Accepting a request after a delay would undo the mode.
3. **Does the operator's page stand alone, or is it a tab in the remote?** Section 6 gives the cost
   of each.
4. **Does a request outlive a restart?** The queue does not. A request list that did would be the
   first evening state the machine keeps.

## 8. What was not examined

- The offline remote's own song mirror, and whether a request can be made while the machine is out
  of reach.
- The television's queue display, and where a count of requests would fit on it.
- The native Android and iOS remotes, beyond the pages they show.
- OpenKJ's automated request mode, and the hosted request service its clients use.
- What a round means when one entry carries no singer name.
