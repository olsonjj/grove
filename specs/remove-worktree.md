# Remove worktree

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins. Removing a linked worktree becomes Grove's second write after fetch; everything else in v1's "repo-changing action" out-of-scope list stays out.

## Problem Statement

I finish with a worktree and have to leave Grove for a terminal to `git worktree remove` it. If it has leftover changes, git refuses and I rerun with `--force`, unsure what I'm throwing away.

## Solution

A "Remove worktree…" action on linked worktrees, in the details pane and the tree row's context menu. It always confirms. A clean worktree gets a plain confirm; one with changes gets a destructive confirm that names what will be discarded. The branch is never touched.

## User Stories

1. As a developer, I want a "Remove worktree…" button in the details pane of a linked worktree, so that I can clean up without a terminal.
2. As a developer, I want "Remove worktree…" in a worktree row's context menu in the tree, so that I can do it from the tree too.
3. As a developer, I want the action absent from the popover and quick switcher, so that destructive actions live in one deliberate place.
4. As a developer, I want the action unavailable on the main worktree, so that I can't remove the repo itself.
5. As a developer, I want the action disabled on a locked worktree with the reason shown, so that I know to unlock it first.
6. As a developer, I want the action on a missing worktree replaced by "Prune", running `git worktree prune` for that repo, so that the stale entry goes away.
7. As a developer, I want a confirm for a clean worktree ("Remove <path>? Branch <name> is kept."), so that a stray click can't delete anything.
8. As a developer, I want a worktree with modified or untracked files to show a destructive confirm with counts ("3 modified and 2 untracked files will be permanently deleted") and a "Discard and remove" button, so that I know exactly what I lose.
9. As a developer, I want both confirms to say ignored files (build output, dependencies) are deleted too, so that nothing surprises me.
10. As a developer, I want the confirm to read the worktree's state fresh when it opens, so that counts aren't stale.
11. As a developer, I want the branch kept, so that removing a worktree never loses commits.
12. As a developer, I want git's error shown and nothing changed when removal fails, so that I can fix it outside Grove.
13. As a developer, I want the tree to update within the usual 2 s and the selection to move to the next row, so that I land somewhere sensible.
14. As a developer, I want the removed worktree's last-opened entry dropped, so that settings don't keep stale paths.
15. As a developer, I want Esc / Cancel to be the default in both confirms and the confirm reachable by keyboard, so that it's safe and accessible.
16. As a VoiceOver user, I want the confirm announced with its message and buttons labelled, so that the feature is accessible.

## Implementation Decisions

- **Git writer (core)** — `remove_worktree(repo, path, force)` runs `git worktree remove [--force] <path>` from the main worktree; `prune_worktrees(repo)` runs `git worktree prune`. Errors return git's brief reason, as fetch does.
- **Removal plan (core, pure)** — given a worktree and a fresh status read, returns `NotAllowed(reason)` (main, locked), `Prune` (missing), `Clean`, or `Dirty { modified, untracked }`. `force` is used only for `Dirty`.
- After a successful removal or prune, the repo is reread through the existing single-repo refresh path and `last_opened` drops the path (saved).
- **UI** — details pane button and tree context-menu item call the same action. Confirm is a modal over the main window, Cancel focused by default, destructive styling for Dirty. Removal runs off the main thread; the confirm's button shows progress and the modal closes on completion. Selection moves to the next row (previous if last).

## Testing Decisions

- **Core seam** — real git in temp dirs (`Fixture`): removal plan for main, locked, missing, clean, modified-only, untracked-only; remove clean; remove dirty with force; non-forced remove of dirty fails with git's reason and leaves the worktree; branch survives removal; prune drops a missing entry.
- **Settings** — removal drops `last_opened` entry.
- **UI seam (thin)** — GPUI test app: button/menu absent on main, disabled on locked; clean confirm → removal called without force; dirty confirm shows counts and calls with force; Cancel/Esc does nothing; error shown on failure; selection moves after removal.

## Out of Scope

- Deleting branches.
- Adding worktrees, or any other repo-changing action.
- Removing from the popover or switcher.
- Bulk removal.
- Undo.
