# Pull (fast-forward only)

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins. Pulling becomes Grove's third write after fetch and worktree removal; everything else in v1's "repo-changing action" out-of-scope list (merge, rebase, push, commit, stage) stays out.

## Problem Statement

The details pane tells me a worktree is N behind its upstream, but getting those commits means leaving Grove for a terminal. A plain `git pull` there can merge or rebase, which I may not want — and can leave the repo in a conflicted state I have to clean up.

## Solution

A "Pull" action on the selected worktree in the details pane, which runs `git pull --ff-only`. It only fast-forwards: when the local branch has diverged from its upstream, git refuses and nothing changes. No merge commits, no conflicts, no rebase — a pull either lands cleanly or doesn't happen.

## User Stories

1. As a developer, I want a "Pull" button in the details pane of a worktree that is behind its upstream, so that I can bring in remote commits without a terminal.
2. As a developer, I want the button absent when there is nothing to pull (no upstream, upstream gone, detached HEAD, up to date), so that it's clear when the action applies.
3. As a developer, I want pull to be fast-forward only, so that diverged branches are refused instead of merged — I decide how to reconcile, outside Grove.
4. As a developer, I want git's short error shown in the status bar and nothing changed when pull refuses (diverged, or my local changes would be overwritten), so that I can fix it outside Grove.
5. As a developer, I want a confirmation after a successful pull ("Pulled <branch>"), so that I know what just happened.
6. As a developer, I want the button to show progress ("Pulling…") and refuse a second click while a pull runs, so that I can't start two.
7. As a developer, I want the tree's badge and the pane's sync line to update right after a pull, so that I see it worked without a full refresh.
8. As a developer, I want the recent-commit list to refresh after pulling, so that the new commits are visible.
9. As a developer, I want the action absent from the popover and quick switcher, so that repo-writing actions live in one deliberate place.
10. As a developer, I want the pull to work even when the upstream counts are marked stale (the last fetch failed), so that the badge's freshness doesn't block pulling — pull fetches for itself.
11. As a VoiceOver user, I want the button labelled with what it does and its outcome announced in the status bar, so that the feature is accessible.

## Implementation Decisions

- **Git writer (core)** — `pull(path)` runs `git -C <path> pull --ff-only --quiet`, with the same config overrides as fetch (`maintenance.auto=false`, `gc.auto=0`) and the existing env (`GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`). It's the only new writing command in the git module.
  - **Timeout** — fetch's generous budget (5 minutes), not unbounded and not the short READ one: the network fetch inside pull dominates. Unlike worktree removal, a kill mid-run is recoverable — a fast-forward is either done or the working tree shows partially updated files that `git status` reports as modified; git leaves no `MERGE_HEAD` behind on a killed fast-forward.
  - **Why `--ff-only`, not plain pull** — plain pull merges or rebases on divergence and can park the repo mid-merge; `--ff-only` refuses fast with git's own reason. This keeps Grove from ever creating a merge commit or conflict state.
- **Offering** — the button renders only when all hold: the worktree is present and openable, HEAD is on a branch (not detached), the upstream is tracking (not `None`/`Gone`), and behind > 0 in the last read status.
  - **Stale counts don't block** — a "stale" marker from a failed Fetch All means ahead/behind are as of an older fetch; pull runs its own fetch, so it's always safe to attempt. (`--ff-only` makes even a wrong attempt harmless.)
- **Dirty worktree is not pre-checked** — git itself refuses when the pull would overwrite uncommitted changes, and that error is shown. Changes that the pull wouldn't touch don't block it (git's behavior).
- **UI** — a small button in the details pane's STATUS section beside the Sync field, styled like the section's other buttons. While running it reads "Pulling…" and is disabled; state is per details pane, and a selection change doesn't cancel a running pull (its notice still lands).
- **After success** — the app rereads that one worktree's status through the existing single-worktree refresh path, so ahead/behind, badges and the watcher are all up to date; the recent-commits read re-runs on its own because HEAD changed. The snapshot cache saves as usual.
- **Errors** — git's brief one-line reason (first `fatal:`/`error:` line), surfaced through the details pane's existing notice, with nothing changed.

## Testing Decisions

- **Core seam** — real git in temp dirs (`Fixture`, an origin bare repo plus a teammate clone, as fetch's tests do): pull fast-forwards behind counts to zero; an up-to-date pull succeeds as a no-op; a diverged branch fails with git's fast-forward reason and moves nothing (HEAD, counts, files all unchanged); a local change to a file the incoming commits touch fails with git's overwrite error; a local change to an untouched file doesn't block the pull; no-upstream and detached worktrees fail.
- **UI seam (thin)** — GPUI test app: button absent on no-upstream/gone/detached/up-to-date, present when behind ≥ 1, absent on the popover and switcher; click → "Pulling…" disabled; success posts the confirmation notice and refreshes the worktree's status (badge clears); failure posts the brief error and changes nothing; a click while pulling is refused; a selection change during a pull doesn't cancel it.

## Further Notes

- The behind count comes from the last fetch, so a button shown off a stale count may pull nothing new — the confirmation still says what happened ("Pulled" with an already-up-to-date result reads as up to date).
- Pull takes the status lock briefly when it updates the working tree; the existing status reads tolerate this (stale-read machinery already handles concurrent refreshes, as during Fetch All).
- The button shows off whatever the details pane last read: a worktree pulled in a terminal still shows "behind" in Grove until its next status read (the watcher triggers one within the usual 2 s).
- Fast-forward-only means a worktree with any local commit ("N ahead") can never pull, even when the local commits would merge cleanly; resolving that means a merge or rebase, which stays out of scope.

## Out of Scope

- Merging, rebasing, or any pull that creates commits.
- Pull all worktrees / repos (Fetch All stays the only bulk action).
- Push, commit, stage, branch switching — v1's out-of-scope list stands.
- Offering pull from the tree row's context menu, popover, or switcher.
- Pre-checking the worktree for blocking local changes (git's own refusal is the check).