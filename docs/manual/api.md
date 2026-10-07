# The HTTP API and the network

- **An HTTP API with a WebSocket event stream** covers search, queue, transport, settings, packages,
  wallpapers, demo mode, microphones and audio output.
- **The URL prefix says which routes need the machine's password.** Everything that reconfigures the
  machine is under `/api/v1/admin/`.
- **Outside it, the method and the path set the level.** A read needs nothing. Adding a song and the
  settings patch need the queue level. Every other write needs the control level, except the two
  debug play routes, which are open whenever debugging mounts them.
  `POST /api/v1/login` exchanges a code for a token of its level, and `GET /api/v1/access` says what
  the caller holds.
- **mDNS finds the machine on the network**, and a `/discover` endpoint answers as well.
- **`POST /api/v1/admin/packages` installs a package without a restart.** Removing one through the
  API deletes its `.kmpkg`.
- **`GET /api/v1/songs/book.pdf` returns the song book**, and takes `?language=`, `?package=` and
  `?name=`. `km-pack book` prints one from `.kmpkg` files that no machine has installed.

To set [a package's bank](songs.md), log in first:

```sh
# Songs 1001-1999 instead of 611001-611999, for the volume that gets sung from.
TOKEN=$(curl -s -H 'content-type: application/json' -d '{"password": "<the machine password>"}' \
        http://127.0.0.1:8177/api/v1/admin/login | jq -r .token)
curl -X PUT -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"bank": 1}' http://127.0.0.1:8177/api/v1/admin/packages/brasil/bank
```
