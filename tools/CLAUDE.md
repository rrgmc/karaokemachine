# `tools/`

**Six folders, and they are the stages of the work.** `cmd/` is the commands somebody types, `port/`
builds the native shells, and `platform/` is what Linux, macOS and Windows each ask for. `dist/`
turns a build into something you hand over, `dev/` is the working session, and `setup/` is what a
build here needs.

**Nothing is loose at the top level** — a script with no folder is a script nobody can place.

**Text a user reads is plain application language**, and that includes `--help`, which clap builds
out of the doc comments on a `Cli` struct. That rule is
[`What a user reads is written in plain application language`](../docs/decisions/foundations.md#what-a-user-reads-is-written-in-plain-application-language),
and it reaches here directly rather than through a catalog header: a command line has no catalog to
head.

**Two programs here have a Fluent catalog**: `assets/km-admin` and `km-package-builder`, each in
`i18n/` beside its own templates. **A new page string is a key.** The parity tests in that program's
`words.rs` fail the build on three faults. The two locales disagree, the markup asks for something no
catalog has, or a template prints a word of its own. `--help` in either program is a doc comment in
English.

Reasoning about a control goes in the `{# #}` or `//` beside it, never on the page.
