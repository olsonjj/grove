# 19: Tree sort by recent activity

**What to build:** Tree header row gets a sort menu after the legend, default Recent, one mode for all repos, remembered across restarts. Recent orders top-level folders and plain branches (mixed, as today) by folder/branch activity, branches inside a folder by branch activity, detached worktrees by activity. Main worktree's branch stays first; worktrees under one branch keep main-first-then-path; detached section stays last. Name gives exactly today's tree. Popover worktrees within each repo follow the same mode. See [spec](../sort-and-pin.md).

**Blocked by:** 17

**Status:** ready-for-agent

- [x] Settings `tree_sort` default `recent`, untouched by Settings Save
- [x] Tree build takes sort mode and last-opened map; Name mode output identical to today
- [x] Recent mode: folder activity = newest branch; collapsed folders keep position; ties by name, unknown after known
- [x] Tree header sort menu switches mode and persists it
- [x] Selection follows its `RowId` through reorders
- [x] Popover worktree order within a repo follows `tree_sort` via the same comparison helpers
- [x] Tests: existing `shape()` tree tests unchanged in Name mode; new Recent-mode cases (folders, nested branches, collapsed, detached, main-first); menu tests for popover order; UI test sort menu persists and keeps selection
