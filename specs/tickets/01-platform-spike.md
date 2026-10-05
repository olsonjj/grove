# 01: Platform spike — menu bar, popover, global shortcuts

**What to build:** A GPUI Kit app that runs as a menu bar app (no Dock icon). Clicking a menu bar icon opens an empty borderless popover window anchored under it. ⌥⌘K shows an empty floating window over whatever app has focus, and it takes keyboard focus. ⌥⌘G shows an empty main window. Proves GPUI's event loop works with a status item and global hotkeys before anything else is built. See spec: Platform shell, Build order.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] App launches with accessory activation policy; no Dock icon
- [x] Menu bar icon visible; click toggles a popover window positioned under the icon; click outside closes it
- [x] ⌥⌘K from another app shows a floating window that receives typed keys; Esc closes it and returns focus to the previous app
- [x] ⌥⌘G from another app shows and focuses the main window
- [x] Switcher window appears within 100 ms of ⌥⌘K (measured, noted in ticket)
- [x] Findings written up: crates used for status item and hotkeys, any GPUI workarounds, go/no-go

## Findings

**Go.** GPUI's event loop coexists with a status item and Carbon global hotkeys with no run-loop hacks; both deliver callbacks on the main thread, forwarded to a GPUI foreground task via `async-channel`.

Crates (all pinned `=`): `gpui-kit 0.7.0` (GPUI = `gpui-pre-* 0.3.7`), `tray-icon 0.26.0` (status item, `default-features = false`), `global-hotkey 0.8.0` (Carbon `RegisterEventHotKey`, no Accessibility permission needed), `objc2 0.6.4` / `objc2-app-kit 0.3.2` (activation policy, frontmost app, screen scale), `async-channel 2.5.0`.

Layout: `crates/grove-app` (bin `grove`); workspace globs `crates/*` so `crates/grove-core` (no GPUI) drops in later.

GPUI workarounds:
- *Superseded 2026-10-04: Grove keeps the Dock icon (Regular policy); swizzle removed.* GPUI hard-codes `setActivationPolicy(Regular)` in `applicationDidFinishLaunching`; flipping to Accessory afterwards left the app `Foreground` (Dock icon) for ~125 ms at launch. Fixed by swizzling `-[NSApplication setActivationPolicy:]` to always pass Accessory, before `application()` (`macos.rs`). Launch now goes BackgroundOnly → UIElement, never Foreground (polled via `lsappinfo`). A bundled `.app` should still set `LSUIElement`.
- Popover and switcher use `WindowKind::PopUp`: a non-activating `NSPanel` at popup level that can become key without activating Grove. So the switcher gets keys while the other app stays active, and Esc hands focus straight back (fallback re-activates the saved previous app if Grove got activated).
- Click-outside = `observe_window_activation` → `remove_window()` when the panel resigns key. A status-item click that arrives <300 ms after such a close is ignored so a second click toggles instead of reopening.
- Esc must call `window.remove_window()` directly; `AnyWindowHandle::update` from inside the same window's handler silently fails (re-entrant).
- First GPUI window costs ~190 ms (release) / ~260 ms (debug) of one-time renderer setup. A hidden 1×1 throwaway window at launch (`warm_up`, ~22–34 ms) removes it.
- `tray-icon` click rects are physical px relative to the main display; divided by the primary screen's backing scale. Multi-monitor menu bars not handled yet.
- `WindowKind::AnchoredPopup` is rejected on macOS in this GPUI; not usable for the popover.
- Accessory apps don't own the menu bar, so ⌘Q needs handling inside our windows (ticket 12). *Moot now that Grove is a Regular app.*

Switcher latency (release, hotkey callback → first render / next frame, logged to stderr as `[grove] latency: …`): 11.6–17.8 ms to first render, 13.5–26 ms to next frame over 6 runs, including the first press after launch. Without warm-up the first press was 190–297 ms.

Verified automatically on macOS 26 / arm64 (release build): `lsappinfo` ApplicationType = UIElement; ⌥⌘K/⌥⌘G via System Events keystrokes with Finder frontmost; typed text shown in the switcher (screenshot + log); Esc → frontmost back to Finder (NSWorkspace); popover opened, toggled and closed by click outside via synthetic CGEvent clicks on the status item, placement checked by screenshot; ⌥⌘G → main window key, `lsappinfo front` = grove.
