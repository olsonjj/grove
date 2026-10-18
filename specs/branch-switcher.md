# Branch switcher

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins. Switching a worktree's branch becomes Grove's fourth write after fetch, worktree removal and pull; branch switching leaves v1's "repo-changing action" out-of-scope list.

## Problem Statement

The details pane tells me which branch a repo is on — and then I have to leave Grove to change it. Looking at a repo means opening a terminal or editor, typing `git switch`, and coming back to see if I got the branch right. For a repo I visit through Grove, the app shows me where I am but can't move me.

## Solution

The branch name at the top of the details pane becomes the control: click it and a searchable dropdown of the repo's local branches opens. Pick one, the worktree switches to it, and the tree, badges and recent commits update at once. Branches already checked out in other worktrees are shown grayed and annotated, so an impossible pick is visible instead of a mystery error. Switching never discards anything: git carries compatible uncommitted changes along and refuses when it can't — Grove surfaces git's word, nothing more.

## User Stories

1. As a developer, I want to change the branch of any worktree from Grove, so that I switch context without leaving the app.
2. As a developer, I want the switcher on every worktree — main or linked — so that there is one rule and no special case.
3. As a developer, I want the branch name at the top of the details pane to be the trigger, so that the affordance sits on the words it changes and I find it immediately.
4. As a developer, I want the dropdown to list all local branches, so that I can go to any branch I already have.
5. As a developer, I want the dropdown to be searchable, so that long branch lists stay navigable.
6. As a developer, I want the branches listed alphabetically with the current one check-marked in place, so that the list position stays stable while I scroll.
7. As a developer, I want branches checked out in another worktree shown disabled with that worktree's folder name, so that I see why the pick is impossible and where the work lives instead.
8. As a developer, I want a detached worktree to offer the dropdown with a placeholder trigger, so that picking a branch is the natural way back onto one.
9. As a developer, I want switching to be refused nothing-and-nobody style: git carries my uncommitted changes when they fit and refuses when they don't, so that switching never loses work.
10. As a developer, I want git's one-line refusal shown in the status bar and nothing changed when the switch fails, so that I can fix it outside Grove.
11. As a developer, I want the trigger to read "Switching…" and the dropdown to be disabled while the switch runs, so that I can't start two.
12. As a developer, I want a selection change not to cancel a running switch, with its notice still landing, so that browsing during a switch costs nothing.
13. As a developer, I want a confirmation ("Switched to `<branch>`") after a successful switch, so that I know what happened.
14. As a developer, I want the tree, badges and the worktree's nesting to update immediately after a switch, so that Grove's own action is faster than its mirror of my terminal.
15. As a developer, I want the recent-commit list to refresh after switching, so that I see the new branch's history.
16. As a developer, I want the selection to stay on the same worktree row after switching, so that I don't lose my place.
17. As a developer, I want a branch created in a terminal to appear in the dropdown without any Grove action, so that the list stays honest — repo-level refreshes already rebuild it, and the watcher's refs and HEAD hits trigger those within the usual 2 s.
18. As a developer, I want the switcher absent from the popover and the quick switcher, so that repo-writing actions live in one deliberate place.
19. As a developer, I want the switcher absent from tree rows and the repo header, so that the thing I mutate is the thing I have selected in the pane.
20. As a keyboard user, I want the trigger reachable by keyboard focus order and the dropdown operable with arrows and Esc, so that switching needs no mouse.
21. As a VoiceOver user, I want the trigger labelled with what it changes and the outcome announced in the status bar, so that the feature is accessible.

## Implementation Decisions

- **Git writer (core)** — `switch(path, branch)` runs `git -C <path> switch --quiet -- <branch>` with the fetch/pull config overrides (`maintenance.auto=false`, `gc.auto=0`) and the existing env (`GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`). It is the git module's fourth declared writing command.
  - **Timeout** — a new 60-second budget, bounded like pull and unlike removal's never-kill. Rationale: a switch interrupted mid-checkout loses nothing — partially checked-out files read as modified, and git holds no merge state (empirically: a kill after git has already written files leaves git refusing a plain rerun — "would be overwritten" — so completing a mid-checkout kill can need the user's hands; nothing is ever lost, which is what the budget needs). Not killed-worthy of `WRITE = MAX`.
- **Branch-list read (core)** — a new read that lists local branch short names in one `for-each-ref refs/heads` call, QUICK-budgeted. Nothing in the git module lists branches today; the snapshot carries only checked-out branches. Branch data elsewhere in the dropdown comes from data Grove already holds: the repo snapshot's worktree list maps each branch to the worktrees holding it.
- **Offer construction (core)** — dropdown items derive from the branch list joined against the snapshot's worktree entries. Each item knows: whether it is the selected worktree's current branch, whether another worktree holds it (with that worktree's path), or neither. No new snapshot field: the mapping is derived per render, not cached.
- **Checked-out elsewhere = disabled, not clickable** — items held by another worktree render grayed and non-interactive, annotated with that worktree's folder name ("in `other-folder/`"). This follows the offer-gating philosophy elsewhere in Grove (Pull absent rather than click-to-refuse; Open disabled on missing worktrees). Git's own refusal ("already checked out at …") remains the backstop for races — a worktree appearing between the snapshot read and the switch — and no error is special-cased beyond `GitError::brief()`.
- **No dirty pre-check** — as with pull, git's own refusal is the check. Switch is not destructive: conflicting changes make it refuse; compatible changes carry over to the target branch (standard git behavior, worth one line in the confirmation's neighborhood in docs, not a warning in the UI).
- **UI** — the branch line at the top of the details pane becomes the trigger, backed by the component library's select: searchable, items alphabetical by name, current branch carrying the select's check icon. Detached worktrees show the identity line as usual ("Worktree · detached HEAD") and give the select a placeholder instead of a current value. The trigger reads "Switching…" while the write runs and refuses a second open; a running switch is single-flight per pane and a selection change doesn't cancel it (its notice still lands) — pull's precedent.
- **Component** — the select comes from the pinned UI kit's built-in select (`searchable(true)`, custom item rows, per-item disabled, check icon, accessibility label). None of Grove's hand-rolled menus are extended for this; the sort menu and ▾-menu patterns stay untouched.
- **After success** — the app rereads that one repo through the existing single-repo refresh path (the only refresh that re-derives branch names from the worktree list, removal's precedent), so the worktree re-nests under its new branch node, badges, the details pane and recent commits all update, and the snapshot cache saves as usual. Selection survives because worktree rows are identified by path.
- **Errors** — git's brief one-line reason, surfaced through the existing notice path into the status bar, with nothing changed.
- **Test faking** — the write and the branch-list read are exposed behind a Global-backed runner (the Puller/Remover pattern), so UI tests fake outcomes without touching the git module.

## Testing Decisions

- **Good tests here** claim external behavior only: what a user sees and what changes in the repo after an action — never which command string ran or how items are stored.
- **Core seam** — real git in temp dirs (the pull/fetch fixture style: origin bare plus clones). Switch succeeds and moves HEAD; uncommitted compatible changes carry over; uncommitted changes conflicting with the target are refused with git's overwrite reason and nothing changed; switching to a branch checked out in another worktree fails with git's refusal; a branch list read returns all local branches including unchecked-out ones; a killed-then-rerun switch completes.
- **UI seam (thin)** — GPUI test app with faked runner. Trigger present at the top of the details pane on the selected worktree; absent from the popover and quick switcher; searchable; alphabetical with the current branch check-marked; checked-out-elsewhere items disabled and annotated; detached worktree shows the placeholder; picking a branch runs the write off the UI thread with "Switching…" disabled; success posts the confirmation and rereads the repo (worktree re-nests, cache saves); failure posts the brief reason and changes nothing; second open refused mid-switch; selection change mid-switch doesn't cancel; VoiceOver label present.

## Out of Scope

- Checking out remote branches (DWIM creation of a local tracking branch) — deliberately deferred; the switch write and dropdown are built so it is a later, separable addition.
- Creating, deleting or renaming branches; detached checkout by SHA.
- Switcher entry points from tree-row context menus, the popover, the quick switcher or a repo header.
- Bulk switching or a "switch repo" action.
- Undo, branch history, or any post-switch reconciliation (merge, rebase, stash).
- A hand-rolled branch-filter UI beyond the component select's built-in search.

## Further Notes

- v1 explicitly listed branch switching out of scope; this spec lifts that for the narrow pick-one-local-branch case only. The write is the least destructive Grove has: git refuses rather than discards, and the only mutation beyond HEAD is nothing — a switch never creates or deletes refs.
- In a repo with no linked worktrees (the common case for these users), the switchee is the main worktree, and the tree re-nests it under the new branch node — which changes which branch row the main worktree heads and can add or empty branch rows. The existing tree-builder already handles this; it is derived from the worktree list.
- The branch list can go stale between renders (a branch deleted in a terminal): the picker shows the snapshot's view, and a stale pick fails with git's own refusal — the backstop, not a special case. Repo-level refreshes triggered by the watcher's refs and HEAD hits correct the list within the usual 2 s.
- The switch confirmation carries the target branch name ("Switched to `feature-x`") even when git considered it up to date — a no-op switch says what happened.
- The 60-second budget leaves the merge-state argument intact: an interrupted switch leaves no merge state, and the working tree's partial state is exactly what git status reports as modified. (Empirical note from ticket 24: git has already written some files by the time its filtered writes stall; a plain rerun then refuses with git's overwrite reason rather than completing, so full recovery can take the user's hands — Grove surfaces git's word and nothing more.)