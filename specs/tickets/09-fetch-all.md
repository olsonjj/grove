# 09: Fetch all

**What to build:** Fetch all in the top bar fetches every remote of every repo with prune, then refreshes ahead/behind and upstream gone. The status bar shows last fetch time. A failed fetch shows the repo and error and keeps last-known counts marked stale. Optional background fetch runs Fetch all every 15 minutes while Grove runs (off by default; toggle lands in settings).

**Blocked by:** 06

**Status:** ready-for-agent

- [ ] Fetch all runs `fetch --all --prune` per repo, concurrently, without blocking the UI
- [ ] Ahead/behind and upstream gone refresh after fetch
- [ ] Status bar shows last fetch time
- [ ] Failure (offline, auth) → status line with repo and error; counts kept and marked stale
- [ ] Background fetch: off by default; when on, every 15 min
- [ ] No other git write commands anywhere
- [ ] Core test: branch deleted on local bare remote shows "upstream gone" after fetch
