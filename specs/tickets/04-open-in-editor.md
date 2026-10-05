# 04: Open in editor

**What to build:** Grove detects installed editors (VS Code, Cursor, Zed, IntelliJ IDEA, Sublime Text) and uses one as default. Enter on a selected worktree or branch row, or the inline Open in editor button on the selected worktree row, opens that worktree in the editor as a new window. The status bar confirms the open or shows why it failed. First end-to-end open path through Launcher planning and execution.

**Blocked by:** 03

**Status:** ready-for-agent

- [ ] Detection finds installed editors by bundle id and CLI; first detected is the default until settings exist
- [ ] Editor CLI used when found, else `open -a <App> <path>`
- [ ] Enter on worktree row opens it; Enter on branch row opens its worktree
- [ ] Inline Open in editor button on selected worktree row
- [ ] Every open launches a new window; no dedupe or focusing
- [ ] Status bar: "Opening <path> in <App>…" on success; reason on failure
- [ ] Configured editor uninstalled → error line (settings link wired when settings exist)
- [ ] Launch planning table tests: each editor with/without CLI, paths with spaces and quotes
