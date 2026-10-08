# 17: Repo sort by recent commits

**What to build:** Sidebar header gets a sort menu ("Recent ▾" / "Name ▾"), default Recent, remembered across restarts. Recent orders non-pinned repos by repo activity, newest first; Name is today's order. Activity in this ticket is HEAD commit time only (edits and opens come in 18). Order updates live as commits land, opens in the saved order from cache, and keeps the selected repo selected and visible. Popover repo groups follow the same mode. Fetch never changes the order. See [spec](../sort-and-pin.md).

**Blocked by:** 16

**Status:** ready-for-agent

- [x] Git reader's worktree status read also returns HEAD committer time (`None` on unborn branch / failure)
- [x] Snapshot worktrees carry `head_time`; old cache without it still loads
- [x] `SortMode` (`Recent | Name`, snake_case); settings `repo_sort` default `recent`, untouched by Settings Save
- [x] Single activity-time helper (max of available signals) used by repo order; repo activity = newest worktree activity
- [x] Repo order Recent mode: newest first, ties by name, unknown after known; pinned group unaffected
- [x] Sidebar sort menu switches mode and persists it
- [x] Selection survives live reorder; selected row scrolled into view
- [x] Popover repo groups follow `repo_sort`
- [x] Tests: commit advances `head_time`; fetch from local bare remote leaves it unchanged; Watcher updates it within 2 s; repo order with `GIT_COMMITTER_DATE`-controlled commits in both modes; UI test sort menu persists and keeps selection
