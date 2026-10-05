# 11: Menu bar popover

**What to build:** Clicking the Grove menu bar icon opens a popover listing every worktree grouped by repo, each with a one-line state and editor/terminal icon buttons. Header has Search (opens the quick switcher) and Fetch all; footer has Open Grove (⌥⌘G), Settings… and Quit. After an action a confirmation line shows and the popover closes after a short delay.

**Blocked by:** 01, 06, 09, 10

**Status:** ready-for-agent

- [ ] Worktrees grouped by repo; one-line state ("3 changed · ↑2 ↓1", "main worktree · clean", "detached @ a1f3c9e")
- [ ] Uncommitted changes in accent color plus words
- [ ] Editor and terminal buttons per row, accessible labels naming branch and app
- [ ] Most recently opened worktree highlighted
- [ ] Header: Search, Fetch all. Footer: Open Grove, Settings…, Quit
- [ ] Confirmation line after action; popover auto-closes after short delay
- [ ] Refreshes when shown
