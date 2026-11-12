# 30: Migrate main window chrome, sidebar & sort menus to the cx-aware palette

**What to build:** The main window's structural chrome goes cx-aware: layout (panes, header, top bar, dividers, status link), sidebar list and its rows/visibility affordances, the sort menu, and the branch and removal flows. Still pinned to Grove Dark's values — visuals unchanged.

**Blocked by:** 28

**Status:** ready-for-agent

- [ ] Layout, sidebar (list + rows + visibility), sort menu, branch and removal flows read only the cx-aware palette for new code
- [ ] Existing main-window test suites (layout, badges, visibility, pane, removal, details) pass unchanged
- [ ] No visual change
- [ ] `just check` green
