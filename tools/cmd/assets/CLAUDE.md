# `tools/cmd/assets/` — a second cargo workspace

**This is not part of the root workspace.** It holds `km-wallpaper-pack` and `km-admin`, and it is
excluded from the one above because those two need TLS from `reqwest` and `km-package-builder` needs
it to have none.

**Every command against them names the manifest and picks the member:**

```sh
cargo build --manifest-path tools/cmd/assets/Cargo.toml -p km-wallpaper-pack
```

A bare `cargo build` from the repository root does not build these, and `-p km-admin` from there does
not resolve. Its build products land in `tools/cmd/assets/target/`, which is separately gitignored.

**Use `--release` for `km-wallpaper-pack`.** A debug run over a full cache took 78 minutes against
single-digit ones optimized.

**`km-admin` serves the machine's own page rather than a copy of it.** It is a *host* for
`crates/machine/km-admin-pages`, implementing that crate's traits over the HTTP client in
`src/machine.rs` — so *This machine*, Songs, Pictures and Sound are the machine's own markup, and what
lives here is the half the machine must never have: finding pictures, fetching banks, and choosing
which machine to talk to. See [`docs/architecture/admin.md`](../../../docs/architecture/admin.md).

**It manages the machine's contents too** — the installed packages, the wallpaper rotation and the
banks the machine already has, removes included. **Every request goes through `machine::Call`**, a
verb and a path as one type with no `&str` door, so a new one is a variant rather than a literal and
`every_call_this_program_makes_is_a_route_the_machine_mounts` sweeps them against the machine's own
route table. **An id in a path goes through `one_segment`**: formatting an id straight into a path
lets a delete address another route entirely.

**Everything it serves is under `/admin`**, because that is where the shared markup's links point.
`/` redirects to `/admin/connect`, which is this program's front door: which machine, and the
password for it. **Nothing else answers until a machine is chosen** — `to_the_door` in `src/server.rs`
redirects every other page there, and that middleware is this program's alone, the machine's own
`/admin/` having no such question. The other two of its own pages are `/admin/pictures/find` and
`/admin/sound/fetch`, wrapped in the shared chrome by `Admin::shell`; the door is wrapped by
`Admin::door`, which draws the same `<head>` and no strip and makes no call to the machine. askama
cannot `{% extends %}` across a crate, which is why both seams are wrappers rather than templates.

**The routers declare their paths without that prefix, so every `src`, `action`, `hx-get` and
`Location` spells it by hand** — and `tests/routes.rs` sweeps every one of them against the routes
actually mounted, because a path that drops it draws alt text or a missing page rather than an error.
A path built from a placeholder needs a sample in that file's `SAMPLES`.

**`km-admin` has no stylesheet of its own.** A new class goes in
`crates/machine/km-admin-pages/static/admin.css`, whose last section is this program's half; one file
serves both surfaces. Its two *scripts* are its own and are **declared** to the shared layout with
`Admin::with_scripts` rather than written into a template, so a host's own markup carries no `<link>`
or `<script>` — a tag written into a host's own layout goes wherever that layout goes.

**Its own templates draw from its own Fluent catalog**, `km-admin/i18n/`, beside them. **A new page
string is a key there**, and the parity tests in `src/words.rs` fail the build if the two locales
disagree or if the markup asks for something no catalog has.

**Adding a control to a page here is a question about which side of the seam it is on**: the machine's
own markup lives in `km-admin-pages` and draws from that crate's catalog; this program's searching
lives here and draws from this one.

**Two language controls, and they are on opposite sides of that seam.** *Screen language* on the
*This machine* tab is the shared markup's and sets what the **television** draws in, over
`GET /locale` and `PUT /admin/machine/locale`. The picker on the front door is this program's own and
sets what **these pages** are in, by writing the `km_locale` cookie — which nothing else on this
origin would, there being no remote mounted beside it. Neither heading is just *Language*.

**Three things are English, each for its own reason.** Data arrives as a value and is shown as it
arrives — a bank's note and license from `km-banks`, a provider's terms, a photograph's credit,
`km-wallpaper-pack`'s rejection codes. `--help` is a command line, which
[`What a user reads is written in plain application language`](../../../docs/decisions/foundations.md#what-a-user-reads-is-written-in-plain-application-language)
governs directly and clap builds out of doc comments. And a **job's outcome sentence** is composed
while a job runs, detached from any request, so no locale is in reach — a phase escapes that by
travelling as a key and being worded at render time, and an outcome would need the same shape. See
`job::phase`.
