# 24: Local branch list read and switch writer (core)

**What to build:** the git module behind the branch switcher. One new read lists local branch short names in a single `for-each-ref refs/heads` call (QUICK budget). One new write, `switch(path, branch)`, runs `git -C <path> switch --quiet -- <branch>` with the fetch/pull config overrides (`maintenance.auto=false`, `gc.auto=0`) and env (`GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`) under a new 60-second bounded budget — a switch killed mid-checkout is recoverable (partial files read as modified; a rerun completes), so it gets pull's philosophy, not removal's never-kill. Both are exposed behind a Global-backed runner for UI faking. Failures surface git's brief reason; nothing else changes. Switch becomes the git module's fourth declared writing command. See [spec](../branch-switcher.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] Branch-list read returns all local branch short names, including branches no worktree has checked out
- [x] `switch(path, branch)` runs exactly the command above with the config overrides and env, timing out at 60 s with the existing timeout error shape
- [x] Module doc updated so switch is declared a writer alongside fetch, worktree removal and pull
- [x] Core seam tests against real git in temp dirs (pull/fetch fixture style)
- [x] Test: switch succeeds — HEAD moves to the branch, files update, nothing created or deleted among refs
- [x] Test: uncommitted compatible changes carry over to the target branch
- [x] Test: uncommitted changes conflicting with the target are refused with git's overwrite reason, nothing changed
- [x] Test: switching to a branch checked out in another worktree fails with git's refusal naming that worktree
- [x] Test: switch to the nonexistent branch fails with git's reason, nothing changed
- [x] Test: an interrupted switch followed by a rerun completes cleanly