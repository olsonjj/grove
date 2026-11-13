# 31: Migrate secondary surfaces to the cx-aware palette

**What to build:** Everything outside the main window goes cx-aware: the Settings window (all sections, command form, footer buttons), the menu bar popover, the quick switcher, first-run/setup surfaces and app shell wiring, the version note, and any remaining stragglers (login items, icons, main entry). After this ticket every render path reads the new palette; only the fixed helpers' definitions remain.

**Blocked by:** 28

**Status:** ready-for-agent

- [x] Settings window, popover, switcher, setup/shell and remaining files use the cx-aware palette (120 fixed-color sites across settings_window/switcher/popover/setup; repo-wide grep clean of fixed-color call sites outside theme.rs)
- [x] Settings, popover and switcher test suites pass unchanged
- [x] No visual change (migration preserves values mechanically; Grove Dark registered by 30a keeps runtime colors Grove's)
- [x] `just check` green; a repo-wide search finds no fixed-color helper call sites left outside the theme module (full `cargo test --all-features --locked`; only the pre-existing unrelated `grove-core::freshness` flake fails)
