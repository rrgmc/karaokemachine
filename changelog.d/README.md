# Unreleased changelog entries

**A pull request that changes what somebody with the machine gets adds one file here**, rather than
editing [`CHANGELOG.md`](../CHANGELOG.md). Every pull request that edited the changelog's
`## [Unreleased]` section edited the same few lines, so any two of them conflicted. A new file in a
folder conflicts with nothing. Cutting a release gathers these into `CHANGELOG.md` and deletes them;
see [`RELEASE.md`](../RELEASE.md).

## The file

`changelog.d/<name>.<type>.md`

- **`<name>`** is lowercase letters, digits and hyphens. The branch's name is the usual choice; it
  only has to differ from the other files here.
- **`<type>`** is the changelog section it goes under: `added`, `changed`, `deprecated`, `removed`,
  `fixed` or `security`.
- **It holds the entry exactly as it will read in the changelog**: a list item opening `- ` in the
  first column, continuation lines indented two spaces, and no heading. Two separate changes are two
  items, in one file or in two.

`changelog.d/media-keys.added.md`:

```markdown
- **A remote's media keys control the song.** Play/Pause pauses and resumes, Stop stops, and Next
  skips. This works with a keyboard's media keys too.
```

The register is the changelog's own: one sentence per change, consequence first, naming what
somebody with the machine gets. A crate, a route or a build flag is the shape to catch.

## What reads it

```sh
bash tools/dist/changelog.sh check     # is every file here well formed — task check and CI run this
bash tools/dist/changelog.sh preview   # the section the next release would write
```

A change nobody with the machine would notice — a refactor, a test, a build script — adds nothing
here.
