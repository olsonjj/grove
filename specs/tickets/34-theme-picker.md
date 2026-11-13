# 34: Theme picker in the Settings dialog

**What to build:** A THEME section in Settings (own header, above the version note, inside the existing scrollable body — the fixed-size window doesn't change). A dropdown button shows the current choice and opens gpui-kit's searchable list: search input ("Search themes…"), rows grouped SYSTEM / LIGHT / DARK, the active choice checked. Picking a theme updates the draft **and re-skins every visible window live** — main window, popover, switcher, and the settings window itself. Save persists the choice through the normal path; Cancel/Escape/any window removal reverts to the choice active when the window opened (capturing the persisted choice, not its resolution). Keyboard: arrows + ⏎ pick, Escape closes the list first, a second Escape cancels the dialog. VoiceOver labels follow the existing choice pattern ("<theme name>, <light|dark>[, selected]"). Works while a custom-command form is open.

**Blocked by:** 33

**Status:** implemented and reviewed (e856e19 + 8e2517b); visual pass (box 9) outstanding

- [x] THEME section with dropdown showing the current choice; list is searchable, grouped System/Light/Dark, with checkmark on the active choice (SYSTEM ships structure-only — empty section never renders via the component's zero-row rule, materializes first when ticket 35 fills it; check tracks the committed choice, not the cursor)
- [x] Search filters case-insensitively on theme name (including the Follow System entry once ticket 35 lands)
- [x] Picking applies the theme across all visible windows in real time, including the settings window (same apply_theme/Theme::update/refresh_windows route as launch; pick leaves the draft unpersisted)
- [x] Save persists the choice; it survives dialog reopen and relaunch (ordinary save_preferences path; round-trip via file.load())
- [x] Cancel and Escape revert to the entry choice, even after multiple picks while the window was open (entry captured as stored; every close path funnels through window removal + on_window_closed; unknown stored name reverts to Grove Dark, matching launch)
- [x] Escape closes the open list first; second Escape cancels the dialog (component consumes Escape while the list is open; verified by test)
- [x] ↑/↓/⏎ keyboard nav; accessibility labels on the button and rows per spec story 14 (arrow nav pinned Grove-level in the pick test; rows "<name>, <light|dark>[, selected]")
- [x] Tests: apply-on-pick, cancel-reverts, save-persists (round-trip through the settings file)
- [ ] Visual check via the `qa-shots/` harness: settings dialog showing the picker, plus a before/after cancel-revert pair (pending human pass — `GROVE_QA_REPO=<path> GROVE_QA_SETTINGS=1 cargo run -p grove-app` on a debug build; note: the trigger can't carry Grove's hover tint, component-owned div — focus/open ring only, QA should confirm it reads right)
- [x] `just check` green (fmt ✓, workspace clippy `-D warnings` ✓, grove-app 242/242 ✓, rustdoc ✓; grove-core untouched by this diff — its freshness flake streak is documented on ticket 33)

**Notes from review (two-axis, both reviewers: OK with notes):** empty-section rule verified upstream-parity (gpui-component list/cache.rs zero-row skip; fill test pins ticket-35 revival); entry/global/save state machine verified leak-free with single close-path funnel; trigger styling limitation confirmed to be the component API's only option. Known verification limits: none open beyond the human visual pass.
