# 30: Migrate main window chrome, sidebar & sort menus to the cx-aware palette

**What to build:** The main window's structural chrome goes cx-aware: layout (panes, header, top bar, dividers, status link), sidebar list and its rows/visibility affordances, the sort menu, and the branch and removal flows. Still pinned to Grove Dark's values — visuals unchanged.

**Blocked by:** 28

**Status:** ready-for-agent

- [x] Layout, sidebar (list + rows + visibility), sort menu, branch and removal flows read only the cx-aware palette for new code (91 sites)
- [x] Existing main-window test suites (layout, badges, visibility, pane, removal, details) pass unchanged
- [x] No visual change from the migration itself (values preserved mechanically); note: the app's active theme is gpui-kit's shadcn default until ticket 33 registers Grove Dark, so migrated surfaces differ from the pre-feature look at runtime — tracked as a plan fix below
- [x] `just check` green (full `cargo test --all-features --locked`; only the pre-existing unrelated `grove-core::freshness` flake fails)
