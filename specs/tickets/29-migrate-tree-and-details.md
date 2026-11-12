# 29: Migrate tree & details to the cx-aware palette

**What to build:** The two heaviest render surfaces — the worktree tree view and the details pane (worktree rows, branch labels, change badges, recent-commit rows with their divider, highlight-tinted worktree text) — call the new cx-aware palette instead of the fixed-color helpers. Since only Grove Dark exists so far, the visuals are byte-for-byte today's look; this just moves the calls.

**Blocked by:** 28

**Status:** ready-for-agent

- [ ] Tree view reads only the cx-aware palette (no new fixed-color helper uses introduced)
- [ ] Details view reads only the cx-aware palette
- [ ] Existing tree/details tests pass on context-threaded colors; no visual change
- [ ] `just check` green
