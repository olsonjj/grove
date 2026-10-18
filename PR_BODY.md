## Summary

The details pane's top line is now the branch switcher: a searchable select of the repo's local branches. Picking drives Grove's fourth git write; the repo rereads itself the way removal and pull already do.

```text
<DetailsView> (crates/grove-app/src/details_view.rs)
  header
+   <BranchSelect> (crates/grove-app/src/branch.rs)   # the branch line at the top
-     STATUS: Branch / Upstream / Sync ...
+     STATUS: Upstream / Sync / Working tree          # Branch row lives on the trigger now

BranchSelect
  branch::items(selected, list, worktrees)            # per render: branch list × snapshot worktrees
    list ← for-each-ref read, QUICK        (grove-core git::branches)
    held elsewhere → grayed + "in `other-folder/`", pick vetoed before git
  pick → switch_runner
    background_spawn git::switch, 60 s     (grove-core git::switch)
    notice "Switched to `<branch>`" | git's brief reason
    shell::reread_repo(git_dir)            # re-nests the worktree, badges, commits, cache
  RepoReads (shell.rs)                     # completed rereads → branch list stays honest
```

## Evidence

- **Before:** the pane shows a branch and offers no way to move it — `git switch` in a terminal, then Grove's watcher catches up within 2 s.
- **After:**
  - Core, real git in temp dirs: 10 tests across `tests/branches.rs`, `tests/switch.rs`, `tests/switch_trace.rs` — switch moves HEAD with files updated and refs byte-for-byte untouched; compatible changes carry over; conflicts refused ("would be overwritten by checkout", nothing changed); held-elsewhere refused naming the worktree; nonexistent branch refused naming itself; interrupted switch reruns to completion; trace pins the exact argv (`-c maintenance.auto=false -c gc.auto=0 switch --quiet -- <branch>`).
  - UI (GPUI, headless): 195 pass in the app binary, including the ticket-25 matrix — trigger present/valued on main and linked worktrees, dropdown searchable alphabetical with the check marked on the current branch, grayed/picked-vetoed for held branches, detached placeholder + pick, "Switching…" refusal of a second open, selection change mid-switch, failure notice + trigger restored, and a real-git end-to-end (`switched_worktree_renests_and_selection_stays`): the worktree re-nests under its new branch node, commits re-read, the selection stays on the row.
  - Terminal-created branch lands after a watcher-driven reread (`branch_created_in_a_terminal_lands_after_repo_refresh`), created and deleted both.
  - `just check` fully green (fmt, clippy -D warnings, tests, -D-warnings rustdoc), and the app exercised by hand against real repos.

## Merge Danger

**Door:** two-way

A revert of the branch restores the pane: the writers, the select and the read are additive seams (Globals no fakes default to the real git commands). The one-way-ish residue: `cargo test` gains `libc` from core's existing dep set only, and `shell::reread_repo` gained `pub` visibility — nothing data-shaped.

**Blast Radius:** pane

The details pane for every worktree row, the STATUS layout (Branch row moves onto the trigger), the new `RepoReads` reread-churn (one QUICK git read per repo reread) and one more declared git writer. Tree, popover, quick switcher and the repo header are untouched (absence-tested); the git write is the least-destructive Grove has — git refuses rather than discards, and the switch creates or deletes no refs.
