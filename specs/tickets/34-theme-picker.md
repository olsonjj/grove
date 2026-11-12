# 34: Theme picker in the Settings dialog

**What to build:** A THEME section in Settings (own header, above the version note, inside the existing scrollable body — the fixed-size window doesn't change). A dropdown button shows the current choice and opens gpui-kit's searchable list: search input ("Search themes…"), rows grouped SYSTEM / LIGHT / DARK, the active choice checked. Picking a theme updates the draft **and re-skins every visible window live** — main window, popover, switcher, and the settings window itself. Save persists the choice through the normal path; Cancel/Escape/any window removal reverts to the choice active when the window opened (capturing the persisted choice, not its resolution). Keyboard: arrows + ⏎ pick, Escape closes the list first, a second Escape cancels the dialog. VoiceOver labels follow the existing choice pattern ("<theme name>, <light|dark>[, selected]"). Works while a custom-command form is open.

**Blocked by:** 33

**Status:** ready-for-agent

- [ ] THEME section with dropdown showing the current choice; list is searchable, grouped System/Light/Dark, with checkmark on the active choice
- [ ] Search filters case-insensitively on theme name (including the Follow System entry once ticket 35 lands)
- [ ] Picking applies the theme across all visible windows in real time, including the settings window
- [ ] Save persists the choice; it survives dialog reopen and relaunch
- [ ] Cancel and Escape revert to the entry choice, even after multiple picks while the window was open
- [ ] Escape closes the open list first; second Escape cancels the dialog
- [ ] ↑/↓/⏎ keyboard nav; accessibility labels on the button and rows per spec story 14
- [ ] Tests: apply-on-pick, cancel-reverts, save-persists (round-trip through the settings file)
- [ ] Visual check via the `qa-shots/` harness: settings dialog showing the picker, plus a before/after cancel-revert pair
- [ ] `just check` green
