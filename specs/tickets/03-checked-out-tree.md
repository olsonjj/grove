# 03: Checked-out tree

**What to build:** Selecting a repo in the sidebar shows its checked-out tree: each checked-out branch with its worktree nested beneath, branches with `/` grouped into collapsible prefix folders, detached worktrees in their own section. Keyboard navigation works. Branches without a worktree are never shown.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Only checked-out branches render; each has its worktree nested beneath
- [ ] Branch rows: branch icon, plain row. Worktree rows: blue folder icon, tinted row
- [ ] Legend above the tree explains both icons
- [ ] Prefix folders (`feature/`, `fix/`) collapsible, show branch count, hidden when empty; collapse state persisted per repo
- [ ] Detached worktrees in a bottom section tagged with short SHA
- [ ] Sort alphabetical, main worktree first; "main worktree" tag shown
- [ ] Worktree paths: parent folders dimmed, folder name bright
- [ ] Arrow keys move selection; ←/→ collapse/expand
- [ ] Last selected repo remembered across launches
- [ ] Core tests assert tree shape for prefixes, detached, main-only repo
