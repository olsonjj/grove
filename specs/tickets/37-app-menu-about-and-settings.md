# 37: App menu — About Grove + Settings…

**What to build:** The conventional app-menu entries and the About window (`specs/updater.md`, "Menu" and "About window" decisions). The Grove menu gains: "About Grove" first (a separator after), and "Settings…" (⌘,) that opens the existing Settings dialog via its normal path. "About Grove" opens a small custom gpui window showing the app icon, "Grove {version} (build {n})" from the single version source Settings already reads, and the GitHub repo link (opens in the browser via NSWorkspace). Opening About twice focuses the existing window instead of stacking one. No new menu entries alter the existing Hide/Show/Quit block.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] New menu order: About Grove, separator, Settings… (⌘,), separator, existing Hide/Show/Quit block; a slot between About and Settings… is reserved — ticket 38 inserts "Check for Updates…" there
- [ ] About Grove shows icon, name, version + build (single-sourced), and a repo link that opens via NSWorkspace
- [ ] Repeated About opens focus the existing window; Settings… (⌘,) and the menu item both open the existing Settings dialog
- [ ] App-level tests: About window contents; Settings… action opens the Settings window; repeat-open focus behavior (gpui-kit test dispatch, prior art: Settings dialog tests)
- [ ] `just check` green
