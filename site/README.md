# The website

One hand-written page per language — `index.html`, `pt-BR/index.html` and one `style.css` — published
by GitHub Actions to **<https://rrgmc.github.io/karaokemachine/>**.

**Opening either page from this folder shows broken images.** The nine pictures live in
[`docs/images/`](../docs/images), which
[`tools/dev/screenshots.sh`](../tools/dev/screenshots.sh) and
[`tools/dev/screen-animation.sh`](../tools/dev/screen-animation.sh) regenerate; a second tracked copy would be
a second thing to keep right. `tools/dist/site.sh` assembles the pages and their pictures into one
folder and is what CI runs, so previewing through it previews exactly what gets published.

```sh
tools/dist/site.sh --open     # stage into dist/site and open it
tools/dist/site.sh            # just stage it
```

`dist/` is gitignored, like every other carrier's output.

## Contents

| | |
|---|---|
| `index.html` | the page in English, served at the published root. No script, and every link out absolute to GitHub |
| `pt-BR/index.html` | the same page in Brazilian Portuguese, served at `/pt-BR/`, reaching the stylesheet and the pictures as `../` |
| `style.css` | the only stylesheet, shared by both pages. No webfont, no CDN, no external request of any kind |

The script stages everything else in the published folder: `images/` from `docs/images/`,
`favicon.png` and `icon-512.png` from [`icon/`](../icon), and a `.nojekyll`.

A language is a folder named for its tag, holding one whole page. Adding one is that page, one line
in `PAGES` and one in `LANGS` at the top of `tools/dist/site.sh`, and nothing else. The stylesheet,
the pictures and the icons are shared. The script refuses a page folder nobody told it about, because
a page nothing stages reaches no reader and breaks nothing that would say so.

## Invariants

- **The palette is the machine's, not the page's.** The custom properties at the top of `style.css`
  are copied from `Theme::default()` in
  [`crates/playback/km-display/src/theme.rs`](../crates/playback/km-display/src/theme.rs), with the
  field each came from named beside it. Amber is the color a syllable turns as it is sung, and the
  machine's icon color. Blue is `km-package-builder`'s, green `km-remote`'s and magenta `km-admin`'s,
  which is why two cards and the admin band are not amber. The page's *surfaces* are `icon_ground`
  and `icon_glow` — the violet-into-magenta the four marks stand on — while `--bg` stays
  `theme.background`. `--raised`, `--line`, `--dim` and `--muted` are page-only, have no theme
  source, and are re-derived toward that violet, so they follow when the theme changes.
- **`km-admin`'s heading is `icon_glow` lifted 18% toward white.** Straight, it measures 3.50:1 on
  `--ground` where the other three measure about 10. `icon.rs` makes the same lift for the same
  reason. Both do it where the color is read rather than in `--glow`, which the hero's wash wants
  dark.
- **Each language is a whole page, and the two say the same things.** Somebody writes the prose in
  its own language rather than rendering it word for word out of English. The structure is not free
  to differ, and `tools/dist/site.sh` refuses four things:

  - two pages that disagree about their sections, their pictures or where they send a reader;
  - a page that does not declare its own `lang`;
  - a page that does not name every language in a `<link rel="alternate">`;
  - a page with no link a reader can click to the other.

  The links out stay English, because the repository and the documents behind them are. The script
  cannot see a paragraph that fell behind in words, and reading both pages is what finds one.
- **The hero shows `icon-512.png`**, beside the wordmark the icon's amber `M` is taken from. It is
  the one mark the page may name. It and `favicon.png` are the only icons `tools/dist/site.sh`
  stages, and a link with nothing behind it fails the build. Another would mean editing the script
  and `pages.yml`.
- **It links one download, and that is the release page.** An asset's name carries the version, so a
  link to one stops resolving at the next release. The carriers table says what each package is and
  links none of them. The URL names the repository the page deploys from, because GitHub serves a
  release only to whoever can see its repository. See
  [`The website is one page per language, and it links one download`](../docs/decisions/repository.md#the-website-is-one-page-per-language-and-it-links-one-download),
  and `Who the README is for` beside it for the rule it inherits.
