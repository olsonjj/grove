# 10: Quick switcher

**What to build:** ⌥⌘K from anywhere, or ⌘K in the main window, opens the quick switcher within 100 ms. Typing searches every worktree by branch name, folder name and repo with highlighted matches updating per keystroke. Empty search shows recently opened worktrees. Enter opens in editor, ⌘Enter in terminal, ⌥Enter copies path, Esc closes.

**Blocked by:** 01, 05

**Status:** ready-for-agent

- [ ] ⌥⌘K global and ⌘K in main window open the same switcher
- [ ] Appears within 100 ms
- [ ] Fuzzy search over branch, folder, repo; highlighted match ranges
- [ ] Results show repo, path, change-count badge
- [ ] Empty query lists recents first (last 10 opened, persisted)
- [ ] No matches → "Nothing matches" with hint that branches appear once checked out in a worktree
- [ ] ↑/↓, Enter, ⌘Enter, ⌥Enter, Esc
- [ ] Opens recorded to recents
- [ ] Search unit tests; GPUI keyboard tests assert the right Launcher call per key
