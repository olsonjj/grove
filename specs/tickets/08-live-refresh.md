# 08: Live refresh

**What to build:** Changes made outside Grove show up within 2 seconds with no manual refresh: worktrees added or removed, branches switched, files edited. Everything refreshes when a window opens. The last Snapshot is cached on disk so the main window opens instantly and refreshes in place.

**Blocked by:** 06

**Status:** ready-for-agent

- [x] FSEvents watching on each repo's git dir and each worktree, debounced ~300 ms
- [x] Ignored paths (e.g. node_modules) don't trigger refreshes
- [x] Grove's own git reads don't retrigger the watcher (no loop)
- [x] Worktree add/remove, branch switch, file edit reflected within 2 s
- [x] Full refresh when main window shown
- [x] Snapshot persisted; window renders cached state before refresh completes
- [x] Idle CPU near zero with watcher running
- [x] Core freshness tests for each change type assert update within 2 s
