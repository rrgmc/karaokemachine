# The website

One hand-written page — `index.html` and `style.css` — published by GitHub Actions to
**<https://rrgmc.github.io/karaokemachine/>**.

**Opening `index.html` from this folder shows broken images.** The eight screenshots live in
[`docs/images/`](../docs/images), which
[`tools/dev/screenshots.sh`](../tools/dev/screenshots.sh) regenerates; a second tracked copy would be
a second thing to keep right. `tools/dist/site.sh` assembles the page and its pictures into one
folder and is what CI runs, so previewing through it previews exactly what gets published.

```sh
tools/dist/site.sh --open     # stage into dist/site and open it
tools/dist/site.sh            # just stage it
```

`dist/` is gitignored, like every other carrier's output.

## Contents

| | |
|---|---|
| `index.html` | the page. No script, and every link absolute to GitHub |
| `style.css` | the only stylesheet. No webfont, no CDN, no external request of any kind |

Everything else in the published folder is staged by the script: `images/` from `docs/images/`,
`favicon.png` and `icon-512.png` from [`icon/`](../icon), and a `.nojekyll`.

## Invariants

- **The palette is the machine's, not the page's.** The custom properties at the top of `style.css`
  are copied from `Theme::default()` in
  [`crates/playback/km-display/src/theme.rs`](../crates/playback/km-display/src/theme.rs), with the
  field each came from named beside it. Amber is the color a syllable turns as it is sung and the
  machine's icon color; blue is `km-package-builder`'s, green `km-remote`'s and magenta
  `km-admin`'s, which is why two cards and the admin band are not amber. The page's *surfaces* are
  `icon_ground` and `icon_glow` — the violet-into-magenta the four marks stand on — while `--bg`
  stays `theme.background`. `km-admin`'s heading is `icon_glow` **lifted 18% toward white**, because
  straight it measures 3.50:1 on `--ground` where the other three measure about 10 — the same lift
  `icon.rs` makes for the same reason, and for the same reason it is done where the color is read
  rather than in `--glow`, which the hero's wash wants dark. `--raised`, `--line`,
  `--dim` and `--muted` are page-only, have no theme source, and are re-derived toward that violet.
  If the theme changes, these follow.
- **The hero shows `icon-512.png`**, beside the wordmark the icon's amber `M` is taken from. It is
  the one mark the page may name: it and `favicon.png` are the only icons `tools/dist/site.sh`
  stages, and a link with nothing behind it fails the build. Another would mean editing the script
  and `pages.yml`.
- **It links one download, and that is the release page.** An asset's name carries the version, so a
  link to one stops resolving at the next release; the carriers table says what each package is and
  links none of them. The URL names the repository the page deploys from, because GitHub serves a
  release only to whoever can see its repository. See
  [`The website is one page, and it links one download`](../docs/decisions/repository.md#the-website-is-one-page-and-it-links-one-download),
  and `Who the README is for` beside it for the rule it inherits.
