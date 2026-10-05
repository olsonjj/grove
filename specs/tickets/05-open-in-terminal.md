# 05: Open in terminal

**What to build:** Grove detects installed terminals (Terminal, iTerm2, Warp, Ghostty, Kitty). ⌘Enter, or the inline Open in terminal button, opens a new tab or window in the default terminal with its working directory set to the worktree. Opening the same worktree again opens another tab, so several Claude Code sessions can run side by side.

**Blocked by:** 04

**Status:** ready-for-agent

- [x] Detection finds installed terminals; first detected is the default until settings exist
- [x] Tab vs window setting honored where the terminal supports both
- [x] Terminal and iTerm2 tabs via AppleScript; if Automation permission is denied, fall back to a new window via `open -a` and show a status-line hint
- [x] Working directory is the worktree path in every terminal
- [x] ⌘Enter and inline button both work; repeated opens create new tabs/windows
- [x] Status bar confirms or shows failure reason
- [x] Launch planning table tests: each terminal × tab/window, paths with spaces and quotes

Checked items are verified in plans, stubs and GPUI tests, not against the real terminal apps (see Notes).

## Notes

- Core: `Terminal` enum + `TargetKind::Terminal`, `Role { Editor, Terminal }`, `default_target(role, …)` (`default_editor` / `default_terminal` wrap it), `NoEditor` → `NoTarget`. `LaunchPlan::AppleScript { script, cwd, fallback }`; `execute` → `Result<Outcome, LaunchError>` with `Outcome::{Opened, FellBack(Fallback), TimedOut}` and `Fallback::{Denied(Automation|Accessibility), Unsupported(msg), NotRunning, Failed(msg)}`. `execute_with(plan, env, &Runner { osascript, osascript_wait })` for tests.
- Detection: same mechanism as editors; also looks in `/System/Applications/Utilities`, `/Applications/Utilities`. CLIs: `ghostty`, `kitty` (PATH or bundle `Contents/MacOS/*`). Ids: `terminal`, `iterm2`, `warp`, `ghostty`, `kitty`; order Terminal, iTerm2, Warp, Ghostty, Kitty. This Mac: Terminal + Warp (checked with real `detect()`).
- Mechanisms:
  - Terminal: AppleScript. Window = `do script "cd '<path>'"`. Tab = Terminal has no new-tab command, so: activate, wait until frontmost (else error, so ⌘T never goes to another app), ⌘T via System Events, wait for the tab count to rise (else error rather than typing into the current tab), `do script … in selected tab`. **Tabs also need Accessibility permission** (keystrokes); the hint names the right pane. Cold launch → waits ~5 s for the launch window and reuses it instead of making two.
  - iTerm2: AppleScript; tab = `tell current window to set t to (create tab with default profile)`, then `write text` into `current session of t`; window = `create window with default profile`. Cold launch reuses the launch window like Terminal.
  - Warp: `open warp://action/new_tab?path=<pct-encoded>` / `new_window` (documented URI scheme).
  - Ghostty: window = `open -na Ghostty.app --args --working-directory=<path>` (new process; Ghostty's documented macOS way). Tab = AppleScript `new tab … with configuration` + `initial working directory` (Ghostty 1.3+). Not running → script raises 9001 and the `open -na` fallback runs quietly (one window, no notice). Older Ghostty fails to compile the script (-2741/-2740) → window + soft notice "tabs need Ghostty 1.3 or later".
  - Kitty: window only (`open -na kitty.app --args --single-instance --directory <path>`, or the CLI directly if no app). Tabs would need `kitty @ launch`, which requires remote control enabled in kitty.conf.
- Execution: AppleScript plans are serialized by a global mutex (rapid opens can't both target one new tab). Any osascript failure runs the fallback (`open -a <Terminal.app|iTerm.app> <path>`): denial → "Opened … in a new <App> window. For tabs, allow Grove in System Settings → Privacy & Security → <pane>"; other errors → "(new tab failed: <msg>)" with osascript's `N:M: execution error:` prefix stripped. -1719 counts as Accessibility only alongside "assistive". osascript still running after 120 s (blocked on the prompt) → `TimedOut`: no fallback, process left running so a late Allow still opens the tab; status "Still waiting for permission to control <App>…".
- App: each launch runs on its own std thread (result via channel), not GPUI's background pool. ⌘Enter → terminal (`OpenRequested.role`); inline icon button `open-terminal` (prototype's `>_` icon, icon-only per prototype, tooltip "Open in <App> (⌘↵)"), inert for missing folders. Editor button left as ticket 04 built it (icon + label).
- Quoting: `cd` + POSIX single-quoted path, then AppleScript-string escaped (`\\`, `\"`, and `\n` `\r` `\t` escapes). Test runs real `osascript -e 'return <literal>'` (no app targeted) and sh/zsh/bash `cd` on dirs with spaces, quotes, backslashes, `$`, backticks, `$(…)`, tab, unicode. fish treats `\\` / `\'` inside single quotes specially, so a path containing a backslash would break under fish (untested).
- macOS attributes Automation to the responsible app: Grove.app when bundled; under `cargo run` it's the terminal running it.
- Verified: plan table tests (Terminal ×{app,no app}, iTerm2, Warp, Ghostty, Kitty ×{app,CLI} × tab/window × 5 tricky paths; cwd = worktree in each) plus script-shape tests (frontmost guard before ⌘T, ~5 s wait, iTerm2 writes into the created tab, Ghostty not-running guard); osascript error classification table; stub osascript tests (success argv/cwd, -1743 → `open -a` stub gets `-a Terminal.app <path>`, Accessibility denial, compile error, not running, other error, fallback failure, missing folder runs nothing, 3 concurrent runs never overlap, timeout → `TimedOut` with no fallback); stub kitty CLI; threaded executor delivers results; GPUI tests (⌘Enter on branch/worktree/folder rows, repeated opens, configured Warp + window, denial hint, timeout, old Ghostty soft notice, cold-launch quiet fallback, tab failure reason, launch failure, uninstalled terminal, inline button click + missing folder).
- **Not verified against real apps** (no real windows / AppleScript against terminals by design): Terminal, iTerm2 and Ghostty scripts never compiled or run against the apps (syntax like `tell current window to set t to (…)` and `new surface configuration` is unchecked); real osascript error texts beyond those documented; Warp URI decoding of `%`-encoded paths; Ghostty `--working-directory`; kitty `--single-instance --directory` forwarding; that `open -a Terminal <folder>` lands in the folder. No live screenshot of the button / status line.
- Default (setting unset): Terminal.app → new window (only Automation needed); iTerm2/Warp/Ghostty → tab. Explicit setting overrides.
