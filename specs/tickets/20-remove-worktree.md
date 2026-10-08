# 20: Remove worktree

**What to build:** "Remove worktree…" on linked worktrees, from the details pane and the tree row context menu. Always confirms; a worktree with changes gets a destructive confirm with modified/untracked counts and removes with `--force`. Main worktree excluded, locked disabled with reason, missing offers "Remove missing entry…" (that entry only). Refused, with the reason, when it holds submodules or another repo (untracked or ignored). The plan is reread when the confirm opens and again on confirm. Branch never deleted. See [spec](../remove-worktree.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] Core git writer: `remove_worktree(repo, path, force)` (also drops a missing worktree's entry, unforced), git's brief reason on error
- [x] Core removal plan: NotAllowed (main, locked, unavailable, submodules, nested repo incl. ignored), Missing, Clean, Dirty { modified, untracked } from fresh presence, admin folder and status reads
- [x] After removal: repo reread, `last_opened` entry dropped and saved
- [x] Details pane button + tree context-menu item; absent on main, disabled with reason on locked, "Remove missing entry…" on missing; absent from popover and switcher
- [x] Confirm modal: clean vs destructive copy, ignored-files note, Cancel default, Esc cancels, keyboard + VoiceOver accessible; plan reread on confirm (refreshed, Cancel refocused, if it changed); progress while running; error shown on failure
- [x] Selection moves to next row (previous if last) after removal
- [x] Tests per spec Testing Decisions
