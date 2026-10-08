# 20: Remove worktree

**What to build:** "Remove worktree…" on linked worktrees, from the details pane and the tree row context menu. Always confirms; a worktree with changes gets a destructive confirm with modified/untracked counts and removes with `--force`. Main worktree excluded, locked disabled with reason, missing offers Prune. Branch never deleted. See [spec](../remove-worktree.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Core git writer: `remove_worktree(repo, path, force)` and `prune_worktrees(repo)`, git's brief reason on error
- [ ] Core removal plan: NotAllowed (main, locked), Prune (missing), Clean, Dirty { modified, untracked } from a fresh status read
- [ ] After removal/prune: repo reread, `last_opened` entry dropped and saved
- [ ] Details pane button + tree context-menu item; absent on main, disabled with reason on locked, Prune on missing; absent from popover and switcher
- [ ] Confirm modal: clean vs destructive copy, ignored-files note, Cancel default, Esc cancels, keyboard + VoiceOver accessible; progress while running; error shown on failure
- [ ] Selection moves to next row (previous if last) after removal
- [ ] Tests per spec Testing Decisions
