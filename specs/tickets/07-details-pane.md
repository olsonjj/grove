# 07: Details pane

**What to build:** The right pane shows everything about the selection: full name and kind ("Branch · has worktree", "Worktree · detached HEAD"), path with Copy path, Open in [default editor] and Open in [default terminal] each with a ▾ menu to use another app once, Reveal in Finder, Copy cd command, upstream, sync in words, working tree status, changed files with status codes, and the last three commits.

**Blocked by:** 05, 06

**Status:** ready-for-agent

- [ ] Name and kind for branch and worktree selections
- [ ] Path with Copy path
- [ ] Primary Open in editor and secondary Open in terminal, each with ▾ listing all detected apps; choice applies to this open only
- [ ] Reveal in Finder; Copy cd command copies a shell-quoted `cd`
- [ ] Upstream, sync ("2 ahead, 1 behind"), working tree status
- [ ] Changed files with M, A, D, ?? codes when present
- [ ] Last 3 commits: short SHA, message, relative time
- [ ] Open buttons disabled for missing/unavailable worktrees
