# `site/`

A *landing* page and deliberately **not** a documentation site: the documents stay as markdown on
GitHub, where they are already read.

**Preview through `tools/dist/site.sh`, not by opening `index.html`** — the screenshots are staged
from `docs/images/` rather than committed here a second time.

**One whole page per language**, English at the root and every other in a folder named for its tag,
sharing one stylesheet and one set of pictures. The script refuses two pages that have drifted apart
in their sections, their pictures or their links out.

The palette is copied from `Theme::default()`, and the page's one download link is the release page
rather than a file. All three are invariants, and they are in [`README.md`](README.md).
