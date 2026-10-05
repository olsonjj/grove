# 04: Open in editor

**What to build:** Grove detects installed editors (VS Code, Cursor, Zed, IntelliJ IDEA, Sublime Text) and uses one as default. Enter on a selected worktree or branch row, or the inline Open in editor button on the selected worktree row, opens that worktree in the editor as a new window. The status bar confirms the open or shows why it failed. First end-to-end open path through Launcher planning and execution.

**Blocked by:** 03

**Status:** ready-for-agent

- [x] Detection finds installed editors by bundle id and CLI; first detected is the default until settings exist
- [x] Editor CLI used when found, else `open -a <App> <path>`
- [x] Enter on worktree row opens it; Enter on branch row opens its worktree
- [x] Inline Open in editor button on selected worktree row
- [x] Every open launches a new window; no dedupe or focusing (new-window flag for code/cursor/subl/zed; IntelliJ and the `open -a` fallback can't force one — app decides)
- [x] Status bar: "Opening <path> in <App>…" on success; reason on failure
- [x] Configured editor uninstalled → error line (settings link wired when settings exist)
- [x] Launch planning table tests: each editor with/without CLI, paths with spaces and quotes

## Notes

- Core: `targets.rs` (App detection: `Editor`, `OpenTarget { kind, app, cli }`, `detect()`, `default_editor()`), `launch.rs` (`plan()` pure, `execute()`). `TargetKind` has only `Editor` now; terminals (05) and custom commands (14) add variants. `LaunchPlan` has only `Spawn`; 05 adds AppleScript.
- Detection: app bundle by known name in `/Applications`, `~/Applications`, `/Applications/Setapp`, plus one `mdfind` query on all known bundle ids (2 s timeout, Trash skipped) for other locations. CLI: resolved login-shell PATH first, else the CLI inside the bundle (VS Code/Cursor `Contents/Resources/app/bin/*`, Zed `Contents/MacOS/cli`, Sublime `Contents/SharedSupport/bin/subl`). ~240 ms incl. PATH resolution; runs in the background at shell start. This Mac: VS Code + Zed, both via `/usr/local/bin` CLIs.
- Plans: `code --new-window`, `cursor --new-window`, `zed --new`, `subl --new-window`, `idea <path>` (no new-window flag exists; IntelliJ focuses an already-open project). No CLI anywhere → `open -a <App.app> <path>`. Chose `open -a` over `open -na`: `-n` starts a second process of single-instance apps, which just forwards to the running one (or fights over session state), so it doesn't guarantee a window either. A bundled CLI counts as "CLI found", so only IntelliJ without a launcher hits `open -a` in practice.
- Path is one argv entry, cwd = worktree; no shell. Execution waits ≤5 s for the CLI to exit: non-zero → `"<cli> failed: <last stderr line>"`; still running → success, reaped in background. Missing folder checked before spawn.
- Default editor: `settings.default_editor` (id: `vscode`, `cursor`, `zed`, `intellij`, `sublime`), else first detected in that order. Configured but gone → status error + "Settings…" placeholder (no action until ticket 13); Grove does not fall back to another editor silently.
- Missing worktree: `RowKind::Worktree.openable = !prunable` (git's prunable flag); Enter and the inline button refuse with a status error, button rendered disabled with a tooltip.
- App: `launcher.rs` global holds detected targets + an injectable executor; tests swap in a recorder. `TreeView` emits `OpenRequested`; `MainWindow` owns the status bar and runs the launch off the main thread (stale results dropped by sequence number).
- Shell: status item / hotkey install failures are logged instead of panicking. (A second instance actually registered ⌥⌘K/⌥⌘G fine here; Carbon allowed it.)
- Dev hook `GROVE_DEV_OPEN=1`: opens the first worktree once editors are detected (screenshots without synthetic input).
- Verified: plan table tests (5 editors × CLI/no-CLI × plain/spaces/quotes+`$`/backticks/unicode); stub `code` integration test records argv `--new-window <path>` and cwd; GPUI tests for Enter on worktree/branch/folder rows, configured editor, launch failure, uninstalled editor, inline button presence + click, missing worktree. Live run with a stub `code` first on PATH (via a fake `$SHELL`): stub recorded `cwd=<…>/code/api`, `--new-window`, `<…>/code/api`; screenshots of inline "Open in VS Code" button and status bar success / failure lines.
- Not verified: real editor windows (stubbed by design); "every open is a new window" therefore only checked as flags in plans, and can't hold for IntelliJ or the `open -a` fallback.

