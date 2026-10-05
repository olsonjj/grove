# 01: Platform spike — menu bar, popover, global shortcuts

**What to build:** A GPUI Kit app that runs as a menu bar app (no Dock icon). Clicking a menu bar icon opens an empty borderless popover window anchored under it. ⌥⌘K shows an empty floating window over whatever app has focus, and it takes keyboard focus. ⌥⌘G shows an empty main window. Proves GPUI's event loop works with a status item and global hotkeys before anything else is built. See spec: Platform shell, Build order.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] App launches with accessory activation policy; no Dock icon
- [ ] Menu bar icon visible; click toggles a popover window positioned under the icon; click outside closes it
- [ ] ⌥⌘K from another app shows a floating window that receives typed keys; Esc closes it and returns focus to the previous app
- [ ] ⌥⌘G from another app shows and focuses the main window
- [ ] Switcher window appears within 100 ms of ⌥⌘K (measured, noted in ticket)
- [ ] Findings written up: crates used for status item and hotkeys, any GPUI workarounds, go/no-go
