# Static assets

**Everything here is compiled into the executable** with `include_str!` and served by an explicit
route in `src/server.rs`. There is no `ServeDir`, no npm, no bundler and no minifier — the pipeline
is `include_str!`.

That is not thrift. This program is a single file somebody copies onto a desktop; a `static/`
directory that had to arrive beside it is 58 KB of ways to be half-installed, which is a failure
`km-package-builder` had and removed.

**Nothing is fetched from a CDN, ever.** The machine this program talks to may be on a network with
no route to the internet — that is one of the cases it exists for — and a page that needs unpkg to
render its own controls would break with nothing on screen to say why.

| File | What it is |
|---|---|
| `htmx.min.js` | htmx 2.0.4, unmodified. Vendored, same copy as the package builder's and the remote's. |
| `htmx-LICENSE.txt` | htmx's license, 0BSD, served at `/static/htmx-LICENSE.txt` because a vendored dependency's terms travel with it. |
| `ui.js` | This program's own, and small on purpose. |
| `icon.png`, `machine.png` | This program's mark and the machine's, for a page showing which machine. |

**There is no stylesheet here.** `km-admin-pages/static/admin.css` is the one both admin surfaces
load, and its last section is this program's half; the reasoning for one file rather than two is at
the top of that section. A new class goes there.

**The two scripts are this program's**, and it **declares** them to the shared layout through
`Admin::with_scripts` rather than writing the tags itself. See `Admin::scripts`.

## Three things htmx will catch you with

The first two cost real time in the package builder; the third is the rule that decides what a
handler answers at all.

- **htmx does not swap a non-2xx response.** A handler that returns 500 with a perfectly good error
  fragment puts nothing on the page at all, so a failure looks like a dead button. `ui.js` listens
  for `htmx:responseError` and `htmx:sendError` and is most of why it exists.
- **`hx-vals` is inherited by a parent walk**, and `hx-disinherit` does not stop it. Two elements
  each contributing the same key produces a duplicated query parameter, axum answers 400, and htmx
  then declines to swap — so the symptom is again a control that does nothing.
- **A status with a sentence in it is for a fragment and nothing else.** Every other control here is
  an ordinary form, so the browser navigates and that status becomes the whole document: one line of
  unstyled text with no chrome and no way back. Those redirect to their own page carrying
  `?kind=&said=`, which `handlers::says` writes and `layout.html` draws. The endpoints that keep a
  status are the four `hx-get` fragments, whose tray `ui.js` fills, and the thumbnail route, which
  answers an `<img src>` where alt text is the failure.
