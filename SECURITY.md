# Security

## Reporting a vulnerability

**Report it privately, through GitHub's form:**
[Report a vulnerability](https://github.com/rrgmc/karaokemachine/security/advisories/new), also on
the repository's Security tab. Do not open a public issue for it.

A report becomes a draft advisory that only you and the maintainer can see. The fix, the advisory and
the release that carries them are prepared there, and published together.

Include what somebody needs in order to reproduce it: the version, the platform, and the steps or
request. If a song or package file triggers it, describe the file's shape or build a small one that
reproduces it. Do not attach a copyrighted song.

## Supported versions

**Only the latest release gets fixes.** A fix ships in a new release, and there are no backports to
older ones.

## What counts

The machine is built for a **home network**. Anybody on that network can search the catalog and queue
a song; that is the design and not a vulnerability. Everything that reconfigures the machine sits
behind its admin password. The reasoning is in
[`Network reach`](docs/decisions/api-and-network.md#network-reach).

In scope, for example:

- reaching an admin route, or anything that changes settings or packages, without the admin password;
- the machine's HTTP API, `km-remote` or `km-package-builder` answering a request from another
  origin or another network in a way they should not;
- a song, package or wallpaper file that crashes a program, reads or writes outside its own folder,
  or runs code;
- a release file that is not what the release page says it is.

Out of scope:

- queueing, skipping or searching from the same network, which needs no password by design;
- a machine whose port was forwarded to the internet while it kept its generated PIN, which the admin
  page warns about until the owner sets a password of their own;
- a problem that only exists in a dependency. Report that to its own project, and open an issue here
  once it has a fixed version.
