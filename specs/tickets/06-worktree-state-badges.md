# 06: Worktree state badges

**What to build:** Tree rows show worktree state as worded badges: ahead/behind (↑2 ↓1), N changed, upstream gone, local only, main worktree, detached @ SHA, locked. Missing worktrees are marked "missing" with Open disabled and a `git worktree prune` hint; worktrees on unmounted drives are dimmed "unavailable". Status reads run in the background per worktree and never write to repos.

**Blocked by:** 03

**Status:** ready-for-agent

- [x] Every git call sets `GIT_OPTIONAL_LOCKS=0` and has a timeout
- [x] Ahead/behind against `@{u}` on any remote; no upstream → "local only", no counts
- [x] Upstream configured but ref missing → "upstream gone"
- [x] Change count from `git status`; badge carries the word, not color alone
- [x] Locked → lock badge, opens normally
- [x] Missing folder → "missing", Open disabled, prune hint
- [x] Unmounted path → dimmed "unavailable", Open disabled
- [x] No "HEAD" badge
- [x] Core tests: dirty worktree, ahead/behind vs local bare remote, fork tracking another remote, no upstream, gone upstream, locked, deleted folder; repo index unchanged after reads
