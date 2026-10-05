# 08: Live refresh

**What to build:** Changes made outside Grove show up within 2 seconds with no manual refresh: worktrees added or removed, branches switched, files edited. Everything refreshes when a window opens. The last Snapshot is cached on disk so the main window opens instantly and refreshes in place.

**Blocked by:** 06

**Status:** ready-for-agent

- [ ] FSEvents watching on each repo's git dir and each worktree, debounced ~300 ms
- [ ] Ignored paths (e.g. node_modules) don't trigger refreshes
- [ ] Grove's own git reads don't retrigger the watcher (no loop)
- [ ] Worktree add/remove, branch switch, file edit reflected within 2 s
- [ ] Full refresh when main window shown
- [ ] Snapshot persisted; window renders cached state before refresh completes
- [ ] Idle CPU near zero with watcher running
- [ ] Core freshness tests for each change type assert update within 2 s
