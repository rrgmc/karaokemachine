# `site/`

A *landing* page per language, and the configuration of the manual. **The manual's words are not
here**: they are `docs/manual/*.md`, which mdBook renders into `docs/` on the site. The other
documents stay as markdown on GitHub.

**Preview through `tools/dist/site.sh`, not by opening `index.html`.** The script stages the
screenshots from `docs/images/`, and it builds the manual. It needs `task mdbook` once per machine.

**One whole page per language**, English at the root and every other in a folder named for its tag.
They share one stylesheet and one set of pictures. The script refuses two pages that have drifted
apart in their sections, their pictures or their links out. **The manual is English only.**

**A manual chapter is one file directly in `docs/manual/`, named in `SUMMARY.md`.** A subfolder
breaks `../images/` on the site, and the script refuses one.

The palette is copied from `Theme::default()`, and the page's one download link is the release page
rather than a file. `manual.css` repeats that palette for the book. All of these are invariants, and
they are in [`README.md`](README.md).
