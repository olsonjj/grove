# 12: App lifecycle

**What to build:** Grove behaves as an always-on app (Dock icon plus menu bar icon). It launches at login (on by default). Closing the main window keeps it running so shortcuts keep working; it quits only from the popover footer, ⌘Q, the app menu or the Dock menu. Before first run finishes, no menu bar icon, shortcut or main window is active.

**Blocked by:** 01, 02

**Status:** ready-for-agent

- [x] Registered as login item via SMAppService, on by default
- [x] Closing main window leaves app, hotkeys and menu bar icon running
- [x] ⌥⌘G reopens main window
- [x] Quit via popover footer, ⌘Q, app menu or Dock menu only
- [x] Menu bar icon and hotkeys inactive until first run completes
