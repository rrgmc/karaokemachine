# The HTTP API and the network

A program of your own can do what the remote does, because the remote uses the same API. The machine
answers it at its own address, under `/api/v1/`.

## What it covers

**An HTTP API with a WebSocket event stream** covers search, queue, transport, settings, packages,
wallpapers, demo mode, microphones and audio output. The event stream says when something changes, so
a client does not poll.

## Who may call what

The API has the same levels as [the setup page](setup.md) gives a phone.

| A route | Needs |
|---|---|
| Any read | Nothing |
| Registering a singer, asking for a song, and taking one back | The request level |
| Adding a song with no singer, and the settings patch | The queue level |
| The host's routes | The host level |
| Every other write outside `/api/v1/admin/` | The control level, which is the default |
| Anything under `/api/v1/admin/` | The machine's password |

**The URL prefix says which routes need the machine's password.** Everything that reconfigures the
machine is under `/api/v1/admin/`. Outside it, the method and the path set the level.

`POST /api/v1/login` exchanges a code for a token of its level, and `GET /api/v1/access` says what
the caller holds.

The two debug play routes are the exception. They are open whenever debugging mounts them.

## Singers and their songs

- **`POST /api/v1/singers` registers a singer**, and the answer carries the singer's id and token.
  `GET /api/v1/singers/rules` says whether a name is required, and the queue order.
- **`POST /api/v1/requests` asks for a song for that singer**, with `singer`, `token` and `number`.
  The machine queues it in the queue order the owner chose. `GET /api/v1/requests` lists every
  request and its state.
- **`POST /api/v1/queue` adds a song with no singer**, and takes `number` and nothing else. It
  refuses a body that names a singer.
- **The host's routes need the host level**: `POST /api/v1/singers/{id}/requests` adds a song
  for any singer. Accepting early, declining and the merge each have a route under
  `/api/v1/requests/` or `/api/v1/singers/`, and `POST /api/v1/evening/reset` starts a new evening.

## Find the machine on the network

**mDNS finds the machine on the network**, and a `/discover` endpoint answers as well.

## Packages and the song book

- **`POST /api/v1/admin/packages` installs a package without a restart.** Removing one through the
  API deletes its `.kmpkg`.
- **`PUT /api/v1/admin/linked-packages` says whether the machine takes
  [linked packages](songs.md#linked-packages)**, with `{"enabled": true}`.
  `GET /api/v1/linked-packages` reads it.
- **`PUT /api/v1/admin/linked-packages/folder` names the folder linked packages play from**, with
  `{"dir": "/tunes/karaoke"}`. The path must be absolute on the machine, and a blank one names no
  folder. `GET` on the same path reads it, with the admin password.
- **`GET /api/v1/songs/book.pdf` returns the song book**, and takes `?language=`, `?package=` and
  `?name=`. `km-pack book` prints one from `.kmpkg` files that no machine has installed.

## An example: give a package shorter numbers

To set [a package's bank](songs.md#give-a-package-shorter-numbers), log in first:

```sh
# Songs 1001-1999 instead of 611001-611999, for the volume that gets sung from.
TOKEN=$(curl -s -H 'content-type: application/json' -d '{"password": "<the machine password>"}' \
        http://127.0.0.1:8177/api/v1/admin/login | jq -r .token)
curl -X PUT -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"bank": 1}' http://127.0.0.1:8177/api/v1/admin/packages/brasil/bank
```
