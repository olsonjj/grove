# 03: Checked-out tree

**What to build:** Selecting a repo in the sidebar shows its checked-out tree: each checked-out branch with its worktree nested beneath, branches with `/` grouped into collapsible prefix folders, detached worktrees in their own section. Keyboard navigation works. Branches without a worktree are never shown.

**Blocked by:** 02

**Status:** ready-for-agent

- [x] Only checked-out branches render; each has its worktree nested beneath
- [x] Branch rows: branch icon, plain row. Worktree rows: blue folder icon, tinted row
- [x] Legend above the tree explains both icons
- [x] Prefix folders (`feature/`, `fix/`) collapsible, show branch count, hidden when empty; collapse state persisted per repo
- [x] Detached worktrees in a bottom section tagged with short SHA
- [x] Sort alphabetical, main worktree first; "main worktree" tag shown
- [x] Worktree paths: parent folders dimmed, folder name bright
- [x] Arrow keys move selection; ←/→ collapse/expand
- [x] Last selected repo remembered across launches
- [x] Core tests assert tree shape for prefixes, detached, main-only repo

## Notes

- Core: `RepoSnapshot::tree(collapsed) -> Tree` (visible rows) and `navigate(tree, selected, key) -> NavEffect` in `grove-core/src/tree.rs`, both pure. App `tree_view.rs` renders and dispatches keys only.
- Prefix folders group by **first segment** (`feature/auth/x` → `feature/` › `auth/x`), as in the prototype. Count = checked-out branches inside.
- Order: main worktree's branch first (full name, even if it has a `/`), then prefix folders and plain branches in one case-insensitive alphabetical list (`chore-deps` before `feature/`). Decided 2026-10-04, differs from prototype.
- → on an open folder or a branch steps to its first child; ← on a child steps to its parent. Collapsing a folder that holds the selection moves selection to the folder.
- Collapse state: `Settings::collapsed[repo]` = sorted prefixes (`"feature/"`); repos with nothing collapsed are dropped from the map.
- Icons: prototype SVGs embedded via our own `AssetSource` (`icons.rs`), falling back to GPUI Kit's bundle.
- UI keyboard tests use `#[gpui_kit::test]` + `TestWindowExt::press` over a canned `RepoSnapshot` (`gpui-kit/test-support` dev-dep).
- Verified by eye (screenshots, `HOME` pointed at a scratch dir so paths show `~`): api-server with main, feature/x, feature/y, fix/z, locked chore-deps, detached bisect, plus idle branches (not shown); restored `fix/` collapse; `last_selected_repo` = second repo opens selected. Keyboard selection/highlight movement checked only through the GPUI tests (no synthetic input on the live app).
- Keyboard selection scrolls into view (`ScrollHandle::scroll_to_item`). GPUI test `keyboard_selection_scrolls_into_view`: 81-row tree in a 320 px window, ↓ to the last row / ↑ back to the first, asserts the selected row lies inside the scroll viewport (fails if the scroll call is removed).
- Collapse persistence end to end: GPUI test `collapse_by_key_is_saved_and_restored` opens the real main window over a temp settings file, collapses/expands `feature/` with ←/→, asserts the file contents after each step, then reopens from the file and finds `feature/` collapsed. `Library` is now its own GPUI global so the main window runs without the tray/hotkey shell.
- That test exposed a race: settings saves run on background threads and could finish out of order, leaving stale state on disk. Saves now carry a generation number and an older save that arrives late is dropped (`shell::Saves`).
- Locked worktree shows no badge yet (ticket 06).
