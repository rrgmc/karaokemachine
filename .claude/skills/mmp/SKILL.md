---
name: mmp
description: Open a pull request for this worktree, merge it on a green CI, then remove the worktree and branch. Only when the owner types /mmp.
disable-model-invocation: true
---

# Pull request, merge and clean

**`/mmp` is the owner asking for the merge.** [`steward`](../steward/SKILL.md) stops a session at a
green pull request, and this skill carries on from there. It covers the work in the current worktree
and nothing else.

**Each step finishes before the next one starts.** A step that fails stops the run, and the last
section says what to report.

## 1. Read where the session stands

```sh
git status --short
git branch --show-current
git log --oneline origin/master..HEAD
```

- **The branch is `master`.** Stop. A session about to change tracked files puts itself in a
  worktree first, and `CLAUDE.md` says how.
- **The worktree holds files this session did not change.** Stop and ask, because they belong to
  somebody else's work.
- **There is nothing to commit and nothing ahead of `origin/master`.** Stop, because there is no
  pull request to open.
- **An open pull request already has this branch as its head.** Use it, and go to step 4.

**A session isolated in a worktree runs each git command on its own.** The harness refuses a git
command joined to another with `&&`, a pipe or a here-document. A commit message therefore goes in
a file in the scratchpad, and `git commit -F` reads it.

## 2. Commit

The session's attribution lines end the message.

Stage the files by name. `git add -A` takes whatever else is in the worktree.

## 3. Check, then push

```sh
task check
```

**A change that no compiler and no test reads may leave clippy and the tests to CI.** That is prose
under `docs/`, a `README.md`, or a skill. A cold compile in a worktree costs about 10 GB and CI runs
both. The nine checks that compile nothing still run:

```sh
for t in lint:local lint:pin lint:version lint:modes lint:mdns lint:cargo lint:labels \
         lint:ci fmt:check; do task "$t" || break; done
```

**Anything else runs the whole of `task check`.** A script, a `Taskfile.yml`, a workflow, a
`Cargo.toml` and a Rust file all count as anything else.

```sh
git push -u origin <branch>
```

## 4. Open the pull request

```sh
gh pr create --base master --label <type> --title "<the commit's subject>" --body-file <file>
```

- **`<type>` is `bug`, `enhancement` or `documentation`.** A workflow adds the program and platform
  labels from the changed paths.
- **The body starts from `.github/pull_request_template.md`.** It says what the change does and the
  fault it answers, and then carries the checklist.
- **A box is ticked only when it is true.** When clippy and the tests did not run, the `task check`
  box stays empty and the line under it says which commands ran.
- **Several commits take a title of their own**, which states the rule the whole branch leaves.

## 5. Wait for `CI ok`

**`master` requires one check, `CI ok`.** It reports last, after the jobs it needs.

**`gh pr checks --watch` returns before `CI ok` exists.** It watches the checks GitHub has listed at
that moment, and `--required` finds none in the first minute. So poll the merge state instead, in
the background:

```sh
for i in $(seq 1 110); do
  state=$(gh pr view <n> --json mergeStateStatus --jq .mergeStateStatus)
  failed=$(gh pr checks <n> 2>&1 | grep -c fail)
  [ "$state" != BLOCKED ] || [ "$failed" != 0 ] && break
  sleep 30
done
gh pr checks <n>
```

- **`CLEAN` with `CI ok` passing** is the only state that goes on to step 6.
- **A failed check stops the run.** Read the job's log, and report the failure with its output. A
  fix is a new commit and a new wait, when the fault is in this branch.
- **`DIRTY` or `BEHIND` stops the run.** The branch conflicts with `master` or is behind it, and
  the owner decides how to bring it up to date.
- **A prose-only change skips the compile jobs**, so `CI ok` passes in about two minutes. A change
  to Rust takes far longer, and the loop above runs again when it ends before the checks do.

## 6. Merge

```sh
gh pr merge <n> --merge
gh pr view <n> --json state,headRefOid,mergeCommit
```

- **`--merge` makes a merge commit**, which is what `master`'s history holds.
- **No `--delete-branch`.** The repository deletes the remote branch itself. The flag reaches only
  the local branch, and from a worktree it fails after the merge has gone through.
- **The state reads `MERGED` before anything is removed.**

## 7. Clean

**Prove the worktree holds nothing the merge lacks, and only then remove it.**

```sh
git status --short        # prints nothing
git rev-parse HEAD        # equals the pull request's headRefOid
```

`ExitWorktree` with `remove` cannot verify the worktree itself, and asks for `discard_changes`.
Both lines above are what make that flag safe. When either one fails, keep the worktree and say why.

Then, in the main checkout:

```sh
git pull --ff-only origin master
git branch -d <branch>
git worktree list
```

- **`git branch -d`, never `-D`.** It refuses an unmerged branch, and that refusal is the check.
- **The pull comes first**, because `-d` compares against the local `master`.
- **Every other worktree stays.** Each belongs to another session's work.

## 8. Report

One short message, with these and nothing else:

- the pull request's link and the merge commit
- what was removed: the worktree, the local branch and the remote branch
- every check that did not run, anywhere

## When the run stops early

**Say which step stopped, what the command printed, and what state the run left behind.** A pull
request that is open and unmerged stays open. A worktree whose work is not merged stays where it is.
