# 29: Migrate tree & details to the cx-aware palette

**What to build:** The two heaviest render surfaces — the worktree tree view and the details pane (worktree rows, branch labels, change badges, recent-commit rows with their divider, highlight-tinted worktree text) — call the new cx-aware palette instead of the fixed-color helpers. Since only Grove Dark exists so far, the visuals are byte-for-byte today's look; this just moves the calls.

**Blocked by:** 28

**Status:** ready-for-agent

- [x] Tree view reads only the cx-aware palette (no new fixed-color helper uses introduced; pre-existing legend chip rgb(0x17222f) deferred to ticket 32 by review)
- [x] Details view reads only the cx-aware palette
- [x] Existing tree/details tests pass on context-threaded colors; no visual change
- [x] `just check` green (full `cargo test --all-features --locked` run; only the pre-existing unrelated `grove-core::freshness::pruned_worktree_outside_code_dir_disappears` flake failed)
