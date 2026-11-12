# 31: Migrate secondary surfaces to the cx-aware palette

**What to build:** Everything outside the main window goes cx-aware: the Settings window (all sections, command form, footer buttons), the menu bar popover, the quick switcher, first-run/setup surfaces and app shell wiring, the version note, and any remaining stragglers (login items, icons, main entry). After this ticket every render path reads the new palette; only the fixed helpers' definitions remain.

**Blocked by:** 28

**Status:** ready-for-agent

- [ ] Settings window, popover, switcher, setup/shell and remaining files use the cx-aware palette
- [ ] Settings, popover and switcher test suites pass unchanged
- [ ] No visual change
- [ ] `just check` green; a repo-wide search finds no fixed-color helper call sites left outside the theme module
