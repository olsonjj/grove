# Sort by recent activity and pinned repos

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins. The v1 decisions "rows sorted alphabetically" (story 25) and the tree-order review decision become the **Name** sort mode, which is no longer the only one.

## Glossary additions

- **Activity time** — when a worktree was last worked in. It's the latest of: when Grove last opened it, its HEAD commit's committer date, and the newest modified time among the files `git status` lists for it. Unknown until at least one of those is read.
- **Branch activity** — the latest activity time among a checked-out branch's worktrees. **Folder activity** is the latest among a prefix folder's branches. **Repo activity** is the latest among a repo's worktrees.
- **Sort mode** — `Recent` (newest activity first) or `Name` (alphabetical, today's order). The repo list and the tree each have their own.
- **Pinned repo** — a repo the user has pinned. Pinned repos are listed in their own group above all the others.

## Problem Statement

The sidebar lists 40+ repos alphabetically. The few repos I work in every day are scattered through it, so I scroll or use ⌘K for repos I touch constantly. The tree has the same problem in a busy repo: the branch I worked on an hour ago is mixed in with ones idle for weeks. Alphabetical order is predictable, but it ignores which repos and branches I actually use.

## Solution

Two complementary features:

1. **Pins.** I pin the handful of repos I always want at hand. They sit in a "Pinned" group at the top of the sidebar, alphabetically, so their order doesn't change and I learn their positions.
2. **Recent sort.** Below the pins, repos are ordered by recent activity (newest first) by default. In the tree, branches are ordered the same way. A small sort control in each list header switches between Recent and Name. The choice is remembered, and the sort control is the only place it's set, so the Settings window doesn't change.

Activity counts work I did anywhere: opening the worktree from Grove, committing, or editing files. A fetch never counts, so Fetch all doesn't reshuffle the list.

## User Stories

### Pinning

1. As a developer, I want to pin a repo from its sidebar row, so that the repos I use daily are always at the top.
2. As a developer, I want a pin button to appear when I hover a sidebar row, so that pinning is one click without cluttering every row.
3. As a developer, I want Pin / Unpin in the repo row's context menu, so that I have a discoverable alternative to the hover button.
4. As a developer, I want pinned repos shown in a "PINNED · N" group above the rest, so that I can see what's pinned and how many.
5. As a developer, I want the pinned group hidden when nothing is pinned, so that the sidebar looks as it does today until I use the feature.
6. As a developer, I want pinned repos sorted alphabetically regardless of sort mode, so that their positions never shift and I can build muscle memory.
7. As a developer, I want a pinned repo to appear only in the pinned group, not twice, so that the list stays as long as my repo count.
8. As a developer, I want the "REPOSITORIES · N" header to keep counting every repo, pinned or not, so that the count still means "repos Grove found".
9. As a developer, I want a pinned repo's row to show a filled pin glyph, so that pinned status is clear in more than position.
10. As a developer, I want to unpin from the row's pin button or context menu, so that pinning is easy to undo.
11. As a developer, I want pins remembered across restarts, so that I set them once.
12. As a developer, I want a pin kept when its repo temporarily disappears (folder moved, volume unmounted, code directory changed), so that it comes back pinned when the repo returns.
13. As a developer, I want pins tied to the repo's location on disk rather than its display name, so that two repos with the same folder name can't be confused.
14. As a developer, I want the selected repo to stay selected when I pin or unpin it, so that pinning doesn't disrupt what I'm looking at.
15. As a developer, I want arrow-key navigation in the sidebar to go straight from the pinned group into the rest, so that the group header doesn't trap the selection.
16. As a developer, I want pinned repos listed first in the menu bar popover too, so that both lists agree on what matters.
17. As a VoiceOver user, I want the pin button labelled "Pin <repo>" / "Unpin <repo>" and pinned rows announced as pinned, so that the feature is accessible.

### Repo sort

18. As a developer, I want the repo list sorted by recent activity by default, so that the repos I'm working in rise to the top without any setup.
19. As a developer, I want a sort control in the sidebar header showing the current mode ("Recent ▾"), so that I always know why the list is in this order.
20. As a developer, I want to switch the repo list to Name order, so that I can scan alphabetically when hunting for a rarely used repo.
21. As a developer, I want my repo sort choice remembered across restarts, so that I don't reset it each time.
22. As a developer, I want a repo's activity to be its most recent worktree activity, so that a linked worktree I'm busy in bumps its repo up.
23. As a developer, I want opening a worktree from any Grove surface (tree, details, switcher, popover) to count as activity, so that "repos I go into" rise even when I don't commit.
24. As a developer, I want committing in a worktree to count as activity, so that work done outside Grove counts too.
25. As a developer, I want editing files in a worktree to count as activity, so that a repo with uncommitted work in progress rises before I commit.
26. As a developer, I want Fetch all and background fetch not to change the order, so that refreshing remote state doesn't reshuffle my list.
27. As a developer, I want repos with equal or unknown activity ordered by name, so that the order is deterministic.
28. As a developer, I want repos whose activity is still unknown (first launch, git error, no worktrees) placed after those with known activity, so that they don't push active repos down.
29. As a developer, I want the order to update live when activity changes, so that it reflects what I just did within the usual 2 s freshness target.
30. As a developer, I want the selected repo to stay selected when the order changes, so that a live reorder never changes what I'm looking at.
31. As a developer, I want the sidebar to keep the selected row visible after a reorder, so that I don't lose sight of it.
32. As a developer, I want the window to open in the saved sort order using cached activity, so that the list doesn't open alphabetically and then jump.
33. As a developer, I want the popover's repo groups to follow the same repo sort mode, so that both surfaces agree.

### Tree sort

34. As a developer, I want a sort control in the tree header row, next to the legend, so that I can order branches independently of repos.
35. As a developer, I want the tree sorted by recent activity by default, so that the branch I just worked on is near the top.
36. As a developer, I want to switch the tree to Name order, so that I get today's alphabetical layout back.
37. As a developer, I want one tree sort mode for all repos, remembered across restarts, so that every repo's tree behaves the same.
38. As a developer, I want the main worktree's branch to stay first in both modes, so that the original clone stays in its usual place.
39. As a developer, I want prefix folders ordered by their newest branch in Recent mode, so that `feature/` rises when I'm working in any feature branch.
40. As a developer, I want branches inside a prefix folder ordered by activity in Recent mode, so that the folder's contents are ordered the same way as the top level.
41. As a developer, I want a collapsed folder to keep its position by its newest branch, so that collapsing doesn't change the order.
42. As a developer, I want worktrees under a branch with several worktrees to keep main-first-then-path order, so that sibling worktrees don't jump around.
43. As a developer, I want the detached section to stay last, with its worktrees ordered by activity in Recent mode, so that the newest bisect is easiest to find.
44. As a developer, I want ties and unknown activity ordered by name, so that the tree is deterministic.
45. As a developer, I want selection to follow the selected row through a reorder, so that keyboard navigation never jumps to a different branch.
46. As a developer, I want the popover's worktrees within each repo to follow the tree sort mode, so that the two surfaces agree.

### Unchanged

47. As a developer, I want the quick switcher unchanged (fuzzy score, recents on empty query), so that typed search behaves exactly as before.
48. As a developer, I want the Settings window unchanged, so that sort and pins stay in the lists they affect.

## Implementation Decisions

### Activity data

- **Git reader** — the worktree status read also returns the HEAD commit's committer time, in Unix seconds. It's `None` on an unborn branch or when the read fails. It runs on the same refresh path as status, so the Watcher already triggers it when HEAD moves.
- **Change time** — the snapshot store stats the paths in the status file list and keeps the newest modified time. Deleted paths are skipped, and nothing is walked beyond the listed paths. This is part of the worktree refresh, not a separate pass.
- **Snapshot** — each worktree gains `head_time` and `changes_time` (both optional Unix seconds). New fields default when they're missing from the persisted cache, so an old cache still loads.
- **Last opened** — settings gain a map from worktree path to the Unix time it was last opened. The existing record-open call updates it next to `recents`. `recents` stays capped at 10 for the switcher, but the map isn't capped. On each full rescan, entries for worktrees no longer in the snapshot are pruned.
- Activity time = max(last opened, `head_time`, `changes_time`). Fetch updates only remote refs, so it never changes any of the three.

### Ordering (core, pure)

- **Repo order** is a pure core function. Given the snapshot, the pinned set, the last-opened map and the repo sort mode, it returns two lists: pinned (by name) and others (by mode). The snapshot itself stays in name order. Ordering is derived from it and never stored, so the cache and other consumers don't change.
- **Tree build** takes the tree sort mode and the last-opened map. Name mode gives exactly today's output. Recent mode orders top-level entries (folders and plain branches mixed, as today) by folder/branch activity, then name. Folder children are ordered by branch activity, and detached worktrees by activity. The main worktree's branch stays first, and worktrees under one branch keep main-first-then-path.
- **Popover** grouping uses the repo order (pinned first, then the others) and the tree sort mode for worktrees within a group, through the same comparison helpers as the tree.
- Ties and unknown activity are ordered by name with the existing case-insensitive comparison. Unknown sorts after any known time.

### Settings

- New persisted fields: `pinned_repos` (list of repo paths), `repo_sort` (default `recent`), `tree_sort` (default `recent`), and `last_opened` (map). All take defaults when missing.
- `SortMode` is an enum `Recent | Name`, serialized in snake_case.
- Settings methods: `toggle_pin(repo_path) -> bool` (whether anything changed), `set_repo_sort`, `set_tree_sort`, and `record_open` extended to stamp `last_opened`. These fields aren't part of `Preferences`, so a Settings window Save never overwrites them. That's the same treatment as `collapsed` and `recents`.

### UI

- Sidebar header: the "REPOSITORIES · N" label on the left and a sort menu button ("Recent ▾" / "Name ▾") on the right. This header is fixed above the scrolling list. If anything is pinned, a "PINNED · N" header and the pinned rows sit below it, also fixed, then a separator, then the scrolling rest.
- Repo row: a pin button appears on hover or selection and is always shown filled on pinned rows. The context menu has Pin / Unpin.
- Tree header: a sort menu button after the legend.
- Selection is held by identity (repo name, `RowId`) as it is today, so reorders keep it. After a reorder, the selected row is scrolled into view.
- Views call the core ordering functions. They do no sorting themselves.

## Testing Decisions

A good test drives behavior through a public seam and asserts the order or state the user would see. It doesn't assert which git command ran or check private helpers.

### Seams (all existing)

1. **Core snapshot seam** — real Git repos in temp dirs (the existing `Fixture`). Committing in a worktree advances its `head_time`. Editing a file sets `changes_time` to at least the file's modified time. Fetch from a local bare remote leaves both unchanged. An old cache without the new fields still loads. Freshness test: a commit updates `head_time` within 2 s with the Watcher running.
2. **Core ordering seam** — the repo-order function and tree build over snapshots built with the fixture. Activity is controlled through commit dates (`GIT_COMMITTER_DATE`) and a canned last-opened map, so tests don't depend on wall-clock time. Tree tests reuse the existing `shape()` helper and add Recent-mode cases next to today's Name cases. Name mode must produce exactly today's shapes. Popover order is covered the same way in the menu tests.
3. **Settings** — plain unit tests: pin/unpin round trip, pins surviving a repo's absence, `record_open` stamping `last_opened`, pruning on rescan, and Settings-window Save leaving pins/sort/last-opened untouched.
4. **UI seam (thin)** — GPUI test app over a canned snapshot. The pin button and context menu toggle the pin and persist it. The sort menu changes order and persists the mode. Selection survives a reorder. Arrow keys cross from the pinned group into the rest.

### Prior art

- Tree order: the existing tree tests (`shape()` assertions over fixture repos).
- Popover order: the existing menu tests.
- Settings persistence through the UI: the existing main-window test that opens a worktree and asserts recents in the settings file.
- Settings Save not clobbering non-preference fields: the existing settings-window tests around recents.

## Out of Scope

- Pinning branches or worktrees.
- Manual (drag) reordering of pins or of the list.
- More sort modes (worktree count, changed count, frequency/"frecency").
- Per-repo tree sort modes.
- Sort or pin controls in the Settings window.
- Changing the quick switcher's ranking.
- Ascending/descending toggles.
- Keyboard shortcut for pin.

## Further Notes

- Pinned-group order is alphabetical for stability. If it turns out you want pins in recent order too, have them follow the repo sort mode. That's a one-line change in the ordering function.
- Recent mode can reorder the list right after an open from Grove. Selection-by-identity keeps that from being disruptive, but watch for jumpiness during daily use. Possible mitigation: delay reorders while the sidebar has focus.
- `changes_time` treats a file touched by a build tool or formatter as activity. That's acceptable because the file shows as changed in Grove anyway.
- `changes_time` doesn't walk: an untracked folder or a submodule counts only by its own modified time, so edits deeper inside may not register. Discarding edits removes their files from the list, so activity can go down. Files dated in the future are ignored.
- In the detached section, a detached main worktree stays first in both modes; the rest go by activity in Recent mode, with ties by path. The popover lists each repo's worktrees in the tree's order, with folders expanded.
- `last_opened` isn't pruned when a rescan couldn't list some repo's worktrees, so a failed read doesn't wipe history.
- Update the PRD's "revisit sort order after use" open question to point here.
