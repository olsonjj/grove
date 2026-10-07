# 10: Quick switcher

**What to build:** ⌥⌘K from anywhere, or ⌘K in the main window, opens the quick switcher within 100 ms. Typing searches every worktree by branch name, folder name and repo with highlighted matches updating per keystroke. Empty search shows recently opened worktrees. Enter opens in editor, ⌘Enter in terminal, ⌥Enter copies path, Esc closes.

**Blocked by:** 01, 05

**Status:** ready-for-agent

- [x] ⌥⌘K global and ⌘K in main window open the same switcher
- [x] Appears within 100 ms
- [x] Fuzzy search over branch, folder, repo; highlighted match ranges
- [x] Results show repo, path, change-count badge
- [x] Empty query lists recents first (last 10 opened, persisted)
- [x] No matches → "Nothing matches" with hint that branches appear once checked out in a worktree
- [x] ↑/↓, Enter, ⌘Enter, ⌥Enter, Esc
- [x] Opens recorded to recents
- [x] Search unit tests; GPUI keyboard tests assert the right Launcher call per key
