---
name: steward
description: How a session drives a pull request in this repository, and when it stops.
---

# Driving a pull request

**A session stops once CI is green on the head and the branch merges cleanly.** The owner reviews
late, often days after the pull request opens. So a session does not poll, subscribe or schedule a
check-in while a pull request waits only for a person.

**A review comment is a new request.** The owner starts a session for it, or asks for it in the
session that opened the pull request.

**Before a push, run `task check`** or the commands it runs, as `CONTRIBUTING.md` lists them.
