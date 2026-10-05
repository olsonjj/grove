# Grove v1 spec

Source PRD: [Grove PRD — Git Branch & Worktree Viewer](https://claude.ai/code/artifact/ed674950-ded3-4dd4-a923-ae944dfd7dde) (rev 43, 2026-10-04). Design prototype: [Grove design prototype](https://claude.ai/artifact/Jqsuc5LGgj5HUsqrarqYA2). Where this spec and the PRD differ, this spec wins and the PRD gets updated.

## Glossary

- **Code directory** — the one folder the user picks. Grove scans only inside it.
- **Repo** — a Git repository found in the code directory (normal or bare).
- **Branch** — a local branch (`refs/heads/*`).
- **Worktree** — a working folder on disk: the original clone, or one made by `git worktree add`. The only thing Grove opens.
- **Main worktree** — the original clone's folder.
- **Checked-out branch** — a branch some worktree has checked out. The only branches Grove shows.
- **Detached worktree** — a worktree on a commit, not a branch.
- **Worktree state** — uncommitted change count, ahead/behind vs upstream, upstream gone, local only, locked, missing, unavailable.
- **Snapshot** — the full in-memory picture: repos → checked-out branches → worktrees → state.
- **Open target** — an editor, terminal or custom command Grove can launch on a worktree.

## Problem Statement

We keep many branches checked out across many repos using `git worktree`. The folders end up scattered across disk. Finding the right one means `git worktree list`, shell history or memory, then `cd` and launching tools by hand, many times a day. Finder can't show which folders are worktrees of which repo, on which branch, in what state.

## Solution

Grove is a macOS-only desktop app (Rust, GPUI Kit) that shows every worktree in the code directory as one tree, with its state at a glance, and opens any of them in our chosen editor or terminal in one action. It's reachable from a main window, a ⌥⌘K quick switcher that works from anywhere, and a menu bar popover. It is read-only: it reflects what we do in Git within 2 seconds and never changes repos, except `git fetch` on request.

Audience: two developers. No distribution, pricing, onboarding for strangers, or analytics.

## User Stories

### First run and code directory

1. As a developer, I want Grove to ask "Where do you keep your code?" on first launch, so that it scans only the folder I choose.
2. As a developer, I want a preview of the repos found and each one's worktree count before I continue, so that I know I picked the right folder.
3. As a developer, I want a clear "No Git repos found in <folder>" message with Choose folder…, so that I can recover from a wrong pick.
4. As a developer, I want Continue disabled until a folder is chosen, so that I can't finish setup half done.
5. As a developer, I want the setup screen to say Grove reads only this folder and that I can change it later, so that I trust what it touches.
6. As a developer, I want a blocking message with install instructions if `git` is missing or too old for worktrees, so that I know why Grove can't work.
7. As a developer, I want no menu bar icon, global shortcut or main window until setup finishes, so that a half-configured app doesn't do anything.
8. As a developer, I want to change the code directory later from settings or the sidebar footer, so that I can reorganize my disk.
9. As a developer, I want a changed code directory to rescan right away and drop repos outside it everywhere, so that every surface matches my choice.
10. As a developer, I want the first-run "no repos" screen shown again if my code directory is moved, renamed, deleted or emptied, so that Grove never shows stale data.

### Discovery

11. As a developer, I want Grove to find repos up to 2 levels below the code directory, so that `~/code/repo` and `~/code/org/repo` both work without a slow full-disk walk.
12. As a developer, I want linked worktrees that sit inside the code directory grouped under their repo instead of listed as separate repos, so that the sidebar shows real repos only.
13. As a developer, I want worktrees a repo keeps outside the code directory still shown, so that nothing Git knows about is hidden.
14. As a developer, I want bare repos listed with only their linked worktrees, so that bare-repo workflows are supported.
15. As a developer, I want Grove to skip descending into a repo's own folders when scanning, so that nested `node_modules` or vendored repos don't appear as repos.

### Main window

16. As a developer, I want a sidebar listing every repo with its worktree count, so that I see all my work at once.
17. As a developer, I want the last selected repo remembered, so that the window opens where I left it.
18. As a developer, I want a tree of only checked-out branches with each worktree nested under its branch, so that the tree stays short even in repos with hundreds of branches.
19. As a developer, I want branches and worktrees to look different (branch icon on a plain row; blue folder icon on a tinted row), so that I never confuse a name with a folder.
20. As a developer, I want a legend above the tree explaining the icons, so that the distinction is learnable.
21. As a developer, I want branches with `/` grouped into collapsible prefix folders with counts, collapse state remembered per repo, so that `feature/` and `fix/` work stays tidy.
22. As a developer, I want detached worktrees in their own section tagged with the short SHA, so that a bisect doesn't get lost.
23. As a developer, I want worktree paths shown with parent folders dimmed and the folder name bright, so that I can scan names quickly.
24. As a developer, I want badges with words (ahead/behind ↑2 ↓1, N changed, upstream gone, local only, main worktree, detached @ SHA, locked), so that state is never shown by color alone.
25. As a developer, I want rows sorted alphabetically with the main worktree first in each repo, so that order is predictable.
26. As a developer, I want inline Open in editor and Open in terminal buttons on the selected worktree row, so that I can open without going to the details pane.
27. As a developer, I want arrow keys to move the selection, ←/→ to collapse/expand, Enter to open in the editor and ⌘Enter to open in the terminal, so that I can use the tree without a mouse.
28. As a developer, I want Enter on a branch row to open its worktree, so that I don't have to step down a row first.
29. As a developer, I want a details pane showing the selection's full name and kind ("Branch · has worktree", "Worktree · detached HEAD"), so that I know exactly what I'm looking at.
30. As a developer, I want the details pane to show the worktree path with Copy path, so that I can paste it elsewhere.
31. As a developer, I want Open in [default editor] as the primary action with a ▾ menu of every configured editor, so that I can use another editor once without changing my default.
32. As a developer, I want Open in [default terminal] with the same ▾ menu for terminals, so that terminals work the same way.
33. As a developer, I want Reveal in Finder and Copy cd command, so that I can hand the folder to other tools.
34. As a developer, I want upstream, sync status in words, and working tree status in the details pane, so that I can decide what to do next.
35. As a developer, I want uncommitted files listed with Git status codes (M, A, D, ??), so that I know what's in flight without opening the worktree.
36. As a developer, I want the three most recent commits (short SHA, message, relative time), so that I remember where I left off.
37. As a developer, I want a top bar with the app name, selected repo, "Jump to… ⌘K", Fetch all and an Open-with settings button, so that the main actions are one click away.
38. As a developer, I want no filter field in the main window, so that it stays a map and finding is the switcher's job.
39. As a developer, I want a status bar with last fetch time, a confirmation after each action, and the main shortcuts, so that I know what just happened.
40. As a developer, I want the main window to show cached state immediately and refresh in place, so that it never opens empty.

### Quick switcher

41. As a developer, I want ⌥⌘K to open the quick switcher from any app, so that I can jump to a worktree without finding Grove first.
42. As a developer, I want ⌘K in the main window and Search in the popover to open the same switcher, so that there's one way to find things.
43. As a developer, I want the switcher on screen within 100 ms of the shortcut, so that it feels instant.
44. As a developer, I want to search worktrees by branch name, folder name and repo with highlighted matches updating on every keystroke, so that a few letters are enough.
45. As a developer, I want each result to show its repo, path and change-count badge, so that I pick the right one.
46. As a developer, I want an empty search to list recently opened worktrees first, so that resuming work is one keystroke.
47. As a developer, I want "Nothing matches" with a hint that branches appear once checked out in a worktree, so that I understand why a branch is missing.
48. As a developer, I want ↑/↓ to move, Enter to open in my editor, ⌘Enter in my terminal, ⌥Enter to copy the path and Esc to close, so that the switcher is fully keyboard driven.

### Menu bar popover

49. As a developer, I want a Grove icon in the menu bar that opens a popover, so that I can open a worktree without bringing the main window forward.
50. As a developer, I want the popover to list every worktree grouped by repo with a one-line state ("3 changed · ↑2 ↓1", "main worktree · clean", "detached @ a1f3c9e"), so that I see state at a glance.
51. As a developer, I want uncommitted changes shown in the accent color plus words, so that dirty worktrees stand out accessibly.
52. As a developer, I want editor and terminal icon buttons on each row with labels naming the branch and app, so that one click opens it and screen readers can describe it.
53. As a developer, I want the most recently opened worktree highlighted, so that I can return to it fast.
54. As a developer, I want Search and Fetch all in the popover header and Open Grove (⌥⌘G) and Settings… in its footer, so that everything is reachable from the menu bar.
55. As a developer, I want a confirmation line after an action and the popover to close after a short delay, so that I know it worked.
56. As a developer, I want to turn the menu bar icon off in settings, with the main window and ⌥⌘K still working, so that I can keep a clean menu bar.

### Opening

57. As a developer, I want Grove to detect installed editors (VS Code, Cursor, Zed, IntelliJ IDEA, Sublime Text) with the command each runs, so that I don't configure them by hand.
58. As a developer, I want Grove to detect installed terminals (Terminal, iTerm2, Warp, Ghostty, Kitty), so that I don't configure them by hand.
59. As a developer, I want to pick one default editor and one default terminal, so that Enter and ⌘Enter do what I expect everywhere.
60. As a developer, I want to choose new tab or new window for terminals that support both, so that terminals open the way I like.
61. As a developer, I want a custom command template with `{path}` (shell-quoted), so that I can use editors Grove doesn't detect.
62. As a developer, I want a Run in terminal checkbox per custom command, on by default, so that `nvim {path}` opens in a terminal tab and `subl {path}` runs in the background.
63. As a developer, I want terminals to start with the working directory set to the worktree, so that I can run commands right away.
64. As a developer, I want every open to launch a new editor window or terminal tab even if the worktree is already open, so that I can run several Claude Code sessions on one worktree.
65. As a developer, I want every launch confirmed in the status line, and failures to show the reason and a link to settings, so that I can fix a broken setup.
66. As a developer, I want Open to fall back to the ▾ menu with an error and a settings link if my configured app was uninstalled, so that I'm never stuck.
67. As a developer, I want Grove to find `git`, `code`, `cursor` and other Homebrew tools even when launched from Finder, so that it behaves like my shell.
68. As a developer, I want Open disabled on missing or unavailable worktrees, so that I don't launch an editor on a folder that isn't there.

### Freshness and fetch

69. As a developer, I want worktrees added or removed in my terminal to appear or disappear within 2 seconds, so that I never refresh by hand.
70. As a developer, I want a worktree to move under its new branch within 2 seconds after I switch branches in it, so that the tree is always right.
71. As a developer, I want change counts to update within 2 seconds of editing files, so that dirty state is trustworthy.
72. As a developer, I want everything refreshed when the main window or popover opens, so that what I see is current.
73. As a developer, I want Fetch all to fetch every remote of every repo, prune deleted remote branches, and refresh ahead/behind, so that "upstream gone" is accurate.
74. As a developer, I want ahead/behind counted against each branch's configured upstream, whatever remote it lives on, and "local only" with no counts when there's no upstream, so that Grove never guesses.
75. As a developer, I want a failed fetch to show the repo and error while keeping last-known counts marked stale, so that offline doesn't wipe my view.
76. As a developer, I want optional background fetch every 15 minutes, off by default, so that counts stay fresh only if I want network traffic.
77. As a developer, I want Grove's own `git status` calls never to write to my repos, so that it can't fight with my tools or loop its own watcher.

### Edge cases

78. As a developer, I want a worktree whose folder I deleted marked "missing" with a hint to run `git worktree prune`, so that I can clean up.
79. As a developer, I want locked worktrees badged but opening normally, so that locks don't get in my way.
80. As a developer, I want worktrees on an unmounted drive shown dimmed as "unavailable", so that they don't vanish or error.
81. As a developer, I want a repo with only its main worktree to show just that branch and folder, so that simple repos look simple.

### App lifecycle

82. As a developer, I want Grove to run as a normal app with a Dock icon plus its menu bar icon, so that it behaves like any other Mac app.
83. As a developer, I want Grove to launch at login (on by default, toggle in settings), so that ⌥⌘K always works.
84. As a developer, I want closing the main window to keep Grove running, and Quit only from the popover footer or ⌘Q, so that shortcuts don't die when I close a window.
85. As a developer, I want ⌥⌘G to bring up the main window from anywhere, so that I can reach it with the menu bar icon off.

### Settings and accessibility

86. As a developer, I want settings to save on Save and discard on Cancel, so that I can back out of changes.
87. As a developer, I want settings stored locally per machine with no syncing or per-repo overrides, so that v1 stays simple.
88. As a developer, I want every action reachable by keyboard, icon buttons labeled, and text at 4.5:1 contrast, so that Grove is accessible.
89. As a developer, I want near-zero idle CPU, so that Grove is free to leave running.
90. As a developer, I want nothing to leave my machine, so that repo names and code stay private.

## Implementation Decisions

### Stack

- Rust, GPUI via GPUI Kit (`gpui-kit` from crates.io, pinned exact version). macOS 14+, Apple silicon and Intel.
- Built with Cargo, bundled as an ad-hoc-signed, non-sandboxed `.app` for the two of us. No notarization, no updater.
- Regular activation policy (Dock icon, ⌘Tab, app menu). Launch at login via the macOS login-item API (SMAppService).
- Dark theme only in v1.

### Modules

Core is a headless library with no GPUI dependency. The app binary is a thin GPUI shell over it.

1. **Shell environment** — resolves the user's login-shell PATH once at startup and on settings save; every subprocess (git, editors, custom commands) uses it. Interface: `resolved_env()`.
2. **Git reader** — the only module that runs `git`. Every call sets `GIT_OPTIONAL_LOCKS=0` and has a timeout. Parses porcelain output into domain types. Commands per the PRD: `worktree list --porcelain`, `for-each-ref refs/heads` (upstream, track), `rev-list --left-right --count <branch>...@{u}`, `status --porcelain=v2`, `log -3`. Fetch is `fetch --all --prune`, the only writing command. Interface: `read_repo(path) -> RepoState`, `read_worktree_status(path) -> WorktreeStatus`, `fetch(repo) -> Result`.
3. **Discovery** — walks the code directory to depth 2. A folder with a `.git` directory, or a bare repo, is a repo and is not descended into. A folder with a `.git` file is a linked worktree; it's attributed to its repo through Git, never listed as a repo. Interface: `discover(code_dir) -> Vec<RepoRef>`.
4. **Snapshot store** — owns the Snapshot. Builds it from Discovery + Git reader, applies targeted refreshes, derives everything the UI shows (tree with prefix folders, sort order, badges, one-line state, search index). Persists the last Snapshot to disk so windows open instantly from cache. Emits change events. Interface: `snapshot()`, `refresh_all()`, `refresh_repo(id)`, `refresh_worktree(id)`, `subscribe()`.
5. **Watcher** — FSEvents via the `notify` crate on each repo's git dir and each worktree. Debounced (~300 ms), ignores paths Git ignores for change counts, maps raw paths to `refresh_repo` / `refresh_worktree` calls. Must meet the 2 s freshness target with near-zero idle CPU.
6. **Search** — fuzzy match over branch name, folder name and repo with match ranges for highlighting; empty query returns recents. Pure function over the Snapshot plus the recents list.
7. **Launcher** — two halves. *Planning* (pure): `plan(target, worktree, settings) -> LaunchPlan`, where a plan is either a process spawn (argv, cwd, env) or an AppleScript (Terminal, iTerm2 tabs). Editors use their CLI if found, else `open -a <App> <path>`. Custom commands substitute shell-quoted `{path}`; Run in terminal on → terminal plan running the command in the worktree; off → background spawn with resolved PATH. *Execution*: runs a plan and reports success or a reason. Never dedupes or focuses existing windows.
8. **App detection** — finds installed editors and terminals by bundle id and CLI presence. Interface: `detect() -> Vec<OpenTarget>`.
9. **Settings** — JSON in Application Support. Fields: code directory, default editor, default terminal, terminal tab/window, custom commands (template, label, run in terminal), show menu bar icon, background fetch, launch at login, recents (last 10 opened worktrees), per-repo collapse state, last selected repo.
10. **Platform shell** — menu bar status item (via `objc2` or `tray-icon`) anchoring a borderless GPUI popover window; global hotkeys ⌥⌘K and ⌥⌘G (via `global-hotkey`); login item; floating quick switcher window that can take focus while Grove is in the background. Shortcuts fixed in v1.
11. **UI** (GPUI Kit) — first-run, main window (sidebar, tree, details, top bar, status bar), quick switcher, popover, settings. Views read from the Snapshot store and call the Launcher; they hold no Git logic.

### Decisions made in review (not yet in the PRD)

- Regular app with Dock icon plus menu bar icon (simpler than accessory; GPUI forces Regular anyway). Launch at login on by default.
- Discovery depth 2.
- Fetch all uses `--prune`.
- `GIT_OPTIONAL_LOCKS=0` on every git call.
- Login-shell PATH resolution for all subprocesses.
- No folder suggestion on first run; the picker opens and the user chooses.
- Enter on a branch row opens its worktree.
- Drop the "HEAD" badge; "main worktree" covers it.
- Recents: last 10, persisted.
- Minimum git 2.20.
- Repo shown by folder name; parent folder added only when names clash.
- Terminal/iTerm2 tabs need macOS Automation permission; if denied, fall back to opening a new window via `open -a` and show a status-line hint.

### Build order

Prototype the riskiest piece first: status item + popover window + global hotkey + floating switcher inside a GPUI app. If GPUI's event loop fights it, the plan changes. Then core (Discovery, Git reader, Snapshot store, Watcher), then Launcher, then UI surfaces.

## Testing Decisions

A good test drives behavior through a public seam and asserts what the user would see. It doesn't assert on internal calls, private structs or which git command ran.

### Seams

1. **Core seam (primary)** — the Snapshot store, given a code directory, produces a Snapshot and refreshes it. Tests build real Git repos and worktrees in temp dirs with the real `git` binary, run the scenario (add worktree, switch branch, edit a file, delete a folder, lock, detach, bare repo, nested org folder, worktree outside code dir, no upstream, gone upstream via a local bare "remote"), and assert on the Snapshot. Freshness tests start the Watcher and assert the Snapshot updates within 2 s. Covers Discovery, Git reader, Snapshot store and Watcher through one interface.
2. **Launch planning seam** — `plan()` is pure. Table tests per target: editors with/without CLI, each terminal tab/window, custom command with run in terminal on/off, paths with spaces and quotes. Execution is not unit tested.
3. **UI seam (thin)** — GPUI's test app context over a canned Snapshot: keyboard flows in the tree and the switcher (arrows, Enter, ⌘Enter, ⌥Enter, Esc) produce the right Launcher call. Only for keyboard behavior; layout is checked by eye against the prototype.

Search gets plain unit tests (pure function). The Platform shell is verified manually against a checklist (hotkey from another app, popover placement, switcher < 100 ms, login item).

### Prior art

None yet; empty repo. Core tests follow the temp-dir-plus-real-git pattern used by tools like `gitoxide` and Zed's git integration tests.

## Out of Scope

- Any repo-changing action: create/switch branches, add/remove worktrees, stage, commit, merge, push. Fetch is the only write.
- Branches without a worktree, remote-only branches, stashes, tags, commit graphs.
- Hosting integrations (PR status).
- Windows and Linux.
- Light theme.
- Configurable shortcuts.
- Per-repo settings overrides, settings sync.
- Distribution, notarization, auto-update, analytics.

## Further Notes

- Open questions carried from the PRD: revisit sort order after use; after v1, decide whether Grove should add/remove worktrees based on how often we leave Grove to do it.
- The PRD still lacks the review decisions listed above; update it when these are confirmed.
- GPUI Kit docs don't cover menu bar items or global hotkeys; that's why the Platform shell prototype comes first.
