# 18: Edits and opens count as activity

**What to build:** A repo with uncommitted edits, or one just opened from any Grove surface, rises in Recent order even without a commit. Activity time becomes the latest of last opened, HEAD commit time and newest changed-file time. See [spec](../sort-and-pin.md).

**Blocked by:** 17

**Status:** ready-for-agent

- [x] Worktree refresh stats paths from the status file list, keeps newest mtime as `changes_time` (deleted paths skipped, no walking); old cache still loads
- [x] Settings `last_opened` map (worktree path → Unix time) stamped by `record_open` alongside `recents`; uncapped; pruned on full rescan for worktrees no longer in the snapshot; untouched by Settings Save
- [x] Activity-time helper includes both new signals; repo and tree orders pick them up with no other changes
- [x] Opening from tree, details, switcher or popover all stamp `last_opened`
- [x] Tests: editing a file sets `changes_time` ≥ its mtime; fetch leaves it unchanged; `record_open` stamps and pruning drops vanished worktrees; repo order with a canned last-opened map; UI test that an open lands in `last_opened`
