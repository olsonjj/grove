# 12: App lifecycle

**What to build:** Grove behaves as an always-on menu bar app. It launches at login (on by default). Closing the main window keeps it running so shortcuts keep working; it quits only from the popover footer or ⌘Q. Before first run finishes, no menu bar icon, shortcut or main window is active.

**Blocked by:** 01, 02

**Status:** ready-for-agent

- [ ] Registered as login item via SMAppService, on by default
- [ ] Closing main window leaves app, hotkeys and menu bar icon running
- [ ] ⌥⌘G reopens main window
- [ ] Quit via popover footer or ⌘Q only
- [ ] Menu bar icon and hotkeys inactive until first run completes
