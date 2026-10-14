# 21: Fast-forward pull in the git writer (core)

**What to build:** the git writer behind the details pane's Pull action: `pull(path)` runs `git -C <path> pull --ff-only --quiet` with fetch's config overrides (`maintenance.auto=false`, `gc.auto=0`) and env (`GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`), under fetch's generous 5-minute budget. A killed run leaves no `MERGE_HEAD` (fast-forward, not merge), so the timeout is safe here. Failures surface git's brief reason; nothing else changes. Pull becomes the git module's third declared writing command. See [spec](../pull.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] `pull(path)` runs exactly `git -C <path> pull --ff-only --quiet` with the config overrides and env above, timing out at fetch's budget with the timeout error shape the writer already uses
- [x] Module doc updated so pull is declared a writer alongside fetch and worktree removal
- [x] Core seam tests against real git in temp dirs (fixture style of the fetch tests: bare origin plus a teammate clone)
- [x] Test: behind worktree fast-forwards — upstream commits land, HEAD moves, nothing merged
- [x] Test: up-to-date pull succeeds as a no-op
- [x] Test: diverged branch is refused with git's fast-forward reason and nothing changed (HEAD, counts, files)
- [x] Test: local change to a file the incoming commits touch is refused with git's overwrite error
- [x] Test: local change to an untouched file doesn't block the pull
- [x] Test: no-upstream and detached-HEAD worktrees fail