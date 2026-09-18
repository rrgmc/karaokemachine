# Development web remote

Two files, no dependencies, no build step.

| File | What it is for |
|---|---|
| `index.html` | Driving the machine by hand. One page, no framework, nothing fetched from the network. |
| `api-walkthrough.sh` | Reading the API. Every endpoint once, with `curl`, in the order a client would use them. |

## Serving it

**`index.html` is compiled into the binary**, so `/dev/` answers in every build with no file staged
anywhere. A directory wins over that copy when there is one, which is how this page gets edited and
reloaded without a rebuild: point `api.dev_remote_dir` at this folder, or just run the machine from a
checkout, which finds it.

**`api.serve_dev_remote` is false by default in every build**, so nothing above happens until
somebody asks — `--dev-remote` for one run, or `"serve_dev_remote": true` under `"api"` for good. See
the `Dev remote in release builds` row in `docs/decisions/remotes.md` for why it stopped being on:
`/` and `/admin/` now answer everything an owner needs, and what is left here is a console.

It still reaches no route the API does not already expose to the same caller — the password is what
gates anything, and that was never the argument. Not publishing a console by default is.

Opening `index.html` straight off disk also works, as long as the API base field is set to the
machine's address and `api.cors_origins` allows the `file://` origin. Serving it from the app is
simpler: same origin, so CORS never enters into it.

## What it is for

Not to be a good remote — to prove the API is complete before a real client exists. It touches every
route in `km_api::routes::SURFACE`, including the ones a polished product surface would hide.

The widgets are in five groups, in the order somebody uses one — **Session**, **Playing**,
**Catalog**, **The machine**, **Diagnostics** — so a new one has a right place to go and the
awkward routes sit together at the bottom rather than wherever they were added. See the
`Dev remote layout` row in `docs/decisions/remotes.md`. What is on the page:

- search with every filter, the lyric timeline for any result, and the single-song read beside it —
  which is a different answer from a search hit, and the one a client uses to reach a song whose
  number it already knows;
- one page of the NDJSON export, with the catalog version off its header. One page and not the
  whole thing: this page proves the endpoint answers and shows the `after=` to ask for next, where
  mirroring a six-figure catalog is a client's job and not a browser tab's;
- queueing by number, reordering and removing **by entry id**;
- the full transport, plus seek — absolute, and relative in 10- and 30-second steps, which reads the
  position back before it jumps rather than trusting the last state event;
- transpose, tempo, volume, and the guide-melody toggle — which is *disabled and explained* when
  melody detection abstained, rather than hidden — and a Reload that reads the settings on their own
  rather than folded into a state, since `settings.read` is a route id an owner can restrict
  separately;
- microphone gain, reverb, echo and mute, labeled as settings-only because mic audio is mixed in
  hardware;
- the SoundFont banks — which one is sounding **and** which one the setting names, since a
  `Ctrl+1`…`Ctrl+9` A/B makes those differ; choosing one; fetching one, with its progress; and
  deleting one, on every row but the bundled bank's. This is the page that asks for `?all=true`, so
  it lists the whole sixty-odd-bank survey and not only the nine the machine offers a singer;
- admin login and logout, beside the API base and the token they go with. A token refused — the
  machine restarted and forgot it, or it expired — clears itself and says so, rather than sitting in
  the field while every admin widget quietly refuses. That happens on the first admin *action*
  rather than the moment the machine comes back, because every `GET` in the surface is a public read
  and there is nothing admin-gated to test a token against without writing something;
- the audio output, wallpapers, and packages (install and uninstall);
- **demo mode** — the switch, whether the settings file holds it, the delay, and the countdown when
  there is one to show. Beside the wallpapers, since those two are what the machine does while
  nobody is using it. The delay has no `persist` box beside it because its route does not take one:
  `PUT /admin/demo/delay` always writes it down. The suitability floor is shown and not editable,
  being the one key of the three with no route behind it;
- **debugging mode** — the switch that mounts the two `debug/play-*` routes and makes the whole
  `debug.` settings section do anything. It takes effect at the next start, because the routes are
  mounted when the router is built;
- `debug/play-file`, and `debug/play-upload` beside it — the same act from the other side of the
  network, so a curator whose corpus is on this box can hear a song on a machine that is not. It is
  *disabled and explained* from `debug_enabled` on `/discover`, which is what that field is for:
  a machine that refuses an upload after taking it closes mid-request, and the sending half reads
  that as a dropped connection rather than as an answer. The checkbox above it is
  `debug.uploads.write`, and it is why the page matters on Android: that build has no command line
  and keeps `settings.json` where only `adb` can reach it, so this was a setting with no way to set
  it. The box follows `/discover` rather than the click, so a tick refused for want of a password
  springs back on its own;
- a raw WebSocket event log, with a transcript of every HTTP call beside it. **The stream comes back
  on its own** — a second doubling to thirty, unbounded — and re-reads everything when it does,
  because this is the page you leave open across a restart. The drop is logged once rather than once
  an attempt, so a machine switched off for an hour does not push the transcript out of the log.
  Reconnect is still there, and pressing it resets the wait.
- **the machine's own log**, in a second pane on a second socket. Not folded into the transcript
  above it, which is a record of what a *client* does and stays one.
  It opens with what the machine has already said, so a page loaded after the interesting moment
  still shows it, and the note above the pane names the filter directive this run is keeping — a
  machine started without `-v` holds nothing below `info`, and a quiet pane has to say which of the
  two reasons that is. Filtered by level and by text in the page, over the lines already there.

Every request is logged with its method, path and status, and errors are shown with their stable
`error` code — so the page doubles as a record of what a client has to do and what it will get back.

If something is missing here, it is missing from the integration tests too. That is the point.

## The end-user remote

Not this, and it exists: `crates/remote/km-remote-pages`, server-rendered askama and htmx, served at
`/` by this same machine. It is not built from files dropped into a `remote/` folder — that seam was
reserved once and retired, because what arrived renders HTML on the server rather than fetching JSON
into a client-side app. See the `Web remote` row in `docs/decisions/remotes.md`.

That page arriving does not retire this one. Where the two overlap, they answer different questions:
the singer's Setup tab offers nine banks, and this page lists the whole survey.
