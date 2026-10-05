# 05: Open in terminal

**What to build:** Grove detects installed terminals (Terminal, iTerm2, Warp, Ghostty, Kitty). ⌘Enter, or the inline Open in terminal button, opens a new tab or window in the default terminal with its working directory set to the worktree. Opening the same worktree again opens another tab, so several Claude Code sessions can run side by side.

**Blocked by:** 04

**Status:** ready-for-agent

- [ ] Detection finds installed terminals; first detected is the default until settings exist
- [ ] Tab vs window setting honored where the terminal supports both
- [ ] Terminal and iTerm2 tabs via AppleScript; if Automation permission is denied, fall back to a new window via `open -a` and show a status-line hint
- [ ] Working directory is the worktree path in every terminal
- [ ] ⌘Enter and inline button both work; repeated opens create new tabs/windows
- [ ] Status bar confirms or shows failure reason
- [ ] Launch planning table tests: each terminal × tab/window, paths with spaces and quotes
