# 13: Settings window

**What to build:** A settings window, opened from the top bar, popover footer or status-line error links. Sections: Code directory (Change… reopens picker and preview; rescans immediately), Editor (detected list with commands, pick default), Terminal (detected list, pick default, tab or window), General (show menu bar icon, background fetch, launch at login). Save applies to every surface; Cancel discards.

**Blocked by:** 05, 09, 11, 12

**Status:** ready-for-agent

- [x] Opens from top bar button, popover Settings…, and error-line links
- [x] Code directory Change… reuses first-run picker and preview; repos outside new folder disappear everywhere
- [x] Sidebar footer shows code directory with Change link to same picker
- [x] Editor and terminal defaults; tab/window option where supported
- [x] Show menu bar icon toggle; off hides icon, main window and ⌥⌘K still work
- [x] Background fetch toggle; launch at login toggle (Save calls `login_item::apply`; hint to allow it in System Settings when status is `RequiresApproval`; Save from a copy that isn't the recorded `login_item_path` should say so, since the old copy may stay a second login item)
- [x] Save applies immediately to all surfaces; Cancel discards
