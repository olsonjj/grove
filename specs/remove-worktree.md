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
6. As a developer, I want the action on a missing worktree replaced by "Remove missing entry…", which after a plain confirm drops only that worktree's entry (`git worktree remove <path>`, no `--force`), so that the stale entry goes away without touching other stale entries (e.g. on an unmounted drive), as repo-wide `git worktree prune` would.
7. As a developer, I want a confirm for a clean worktree ("Remove <path>? Branch <name> is kept."), so that a stray click can't delete anything.
8. As a developer, I want a worktree with modified or untracked files to show a destructive confirm with counts ("3 modified and 2 untracked files will be permanently deleted") and a "Discard and remove" button, so that I know exactly what I lose.
9. As a developer, I want both confirms to say ignored files (build output, dependencies) are deleted too, so that nothing surprises me.
10. As a developer, I want the confirm to read the worktree's state fresh when it opens, and again when I confirm (showing the new state instead of removing if it changed), so that it never deletes more than it listed.
11. As a developer, I want the branch kept, so that removing a worktree never loses commits.
12. As a developer, I want git's error shown and nothing changed when removal fails, so that I can fix it outside Grove.
13. As a developer, I want the tree to update within the usual 2 s and the selection to move to the next row, so that I land somewhere sensible.
14. As a developer, I want the removed worktree's last-opened entry dropped, so that settings don't keep stale paths.
15. As a developer, I want Esc / Cancel to be the default in both confirms and the confirm reachable by keyboard, so that it's safe and accessible.
16. As a VoiceOver user, I want the confirm announced with its message and buttons labelled, so that the feature is accessible.
17. As a developer, I want removal refused, with the reason, when the worktree contains submodules or another Git repository, so that `--force` can't delete history the counts don't show.

## Implementation Decisions

- **Git writer (core)** — `remove_worktree(repo, path, force)` runs `git worktree remove [--force] -- <path>` from the main worktree, with no timeout (killing it mid-delete would leave a half-deleted worktree). Errors return git's brief reason, as fetch does.
- **Removal plan (core)** — from the worktree, its admin folder (`<git dir>/worktrees/<id>`) and a fresh `status -uall`, returns `NotAllowed(reason)` (main, locked, unavailable, submodules, nested repo, unreadable status), `Missing`, `Clean`, or `Dirty { modified, untracked }`. `force` is used only for `Dirty`.
  - Presence is rechecked first: a missing folder that came back is read like any other.
  - Submodules: a changed submodule (`S…` in status) or an admin `modules/` folder (initialized submodules' repos, which `--force` deletes). The admin folder comes from the worktree's `.git` file, else the admin folder whose `gitdir` points back; either may be relative (`worktree.useRelativePaths`) and is resolved lexically.
  - Nested repo: an untracked folder entry (git lists files under `-uall`, so a folder is a repo it won't enter) with a `.git`, or any folder at or under an ignored folder entry (`--ignored=matching`) holding a `.git`; ignored files go even on a plain remove. The walk doesn't follow symlinks and stops at the first hit.
- The plan is reread when the confirm opens and when it's confirmed; a changed plan refreshes the confirm instead of removing.
- After a successful removal, `last_opened` drops the path (saved) and the repo is reread through the existing single-repo refresh path, at app level so closing the window mid-removal doesn't skip it.
- **UI** — details pane button and tree context-menu item call the same action. Confirm is a modal over the main window, Cancel focused by default, destructive styling for Dirty. Removal runs off the main thread; the confirm's button shows progress and the modal closes on completion. Selection moves to the next row (previous if last).

## Testing Decisions

- **Core seam** — real git in temp dirs (`Fixture`): removal plan for main, locked, missing, clean, modified-only, untracked-only, submodules, nested repo; remove clean; remove dirty with force; non-forced remove of dirty fails with git's reason and leaves the worktree; branch survives removal; removing a missing entry leaves other missing entries.
- **Settings** — removal drops `last_opened` entry.
- **UI seam (thin)** — GPUI test app: button/menu absent on main, disabled on locked; clean confirm → removal called without force; dirty confirm shows counts and calls with force; Cancel/Esc does nothing; error shown on failure; selection moves after removal; a worktree changed while the confirm is open refreshes it; one dirtied after the last check is refused by git and kept.

## Further Notes

- Counts come from `git status`, so assume-unchanged and skip-worktree files are invisible to them (git's behavior), though `--force` deletes them. Deleted, renamed and type-changed files count as modified.
- A worktree dirtied between the confirm's last check and git running is refused by git (no `--force` for `Clean`) and kept. A `Dirty` one gaining files in that window loses them.
- The confirm-time reread compares the plan only (counts, not file lists): swapping one changed file for another between opening and confirming passes unnoticed.

## Out of Scope

- Deleting branches.
- Adding worktrees, or any other repo-changing action.
- Removing from the popover or switcher.
- Bulk removal.
- Undo.
