# Vendored assets

**Compiled into the binary** by `include_str!` in `src/server.rs`, which serves them at `/static/`.
They are not shipped as files and this folder is not staged by `tools/dist/cmd.sh` — see the
*Bundling assets* entry in `docs/decisions/`. Editing a file here changes the next build; there is nothing
to copy anywhere afterwards.

Nothing here reaches the network at run time either, which is the rule
`tools/dev/remote/index.html` follows. The curation tool has to work on a machine that holds a corpus
and no internet. An unreachable CDN would leave every button inert with no explanation.

| File | What it is | License |
|---|---|---|
| `htmx.min.js` | [htmx](https://htmx.org) **2.0.4**, unmodified | Zero-Clause BSD, verbatim in `htmx-LICENSE.txt` |
| `style.css` | This tool's own stylesheet | Same as the repository |
| `ui.js` | This tool's own script — **not vendored** | Same as the repository |

`ui.js` is the odd one out, and the folder's title is two thirds true. It is ours, and the update
recipe below does not apply to it.

Its first reason is that htmx does not swap a non-2xx response. A request that fails changes nothing
on the screen, so this turns the error event into a line of text. Beside that it answers the three
things htmx has no opinion about. The box in the table head, the shift-click that ticks a run of
rows, and which page a view change lands on. That is its whole brief, and the whole of this tool's
JavaScript. The head of the file, and the doc comment on `UI_JS` in `src/server.rs`, say what it may
not grow into.

To update htmx, replace the file and its license together and change the version above:

```sh
curl -sSL -o htmx.min.js https://unpkg.com/htmx.org@<version>/dist/htmx.min.js
curl -sSL -o htmx-LICENSE.txt https://raw.githubusercontent.com/bigskysoftware/htmx/v<version>/LICENSE
```
