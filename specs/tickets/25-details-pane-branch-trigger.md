# 25: Branch switch trigger in the details pane

**What to build:** the branch line at the top of the details pane becomes the switcher trigger, backed by the component library's searchable select. Items are the repo's local branches, alphabetical, current branch check-marked; branches held by another worktree render disabled and annotated with that worktree's folder name ("in `other-folder/`"); detached worktrees get a placeholder trigger instead of a current value. Picking a branch runs the switch off the UI thread with the trigger reading "Switching…" and disabled (single-flight; a selection change mid-switch doesn't cancel it and its notice still lands). On success a "Switched to `<branch>`" confirmation posts and that one repo is reread through the existing single-repo refresh path, so the worktree re-nests under its new branch node, badges, the pane and recent commits update, and the snapshot cache saves. On failure only git's brief reason is shown in the status bar and nothing changes. The trigger exists nowhere else (absent from popover, switcher, tree rows and the repo header). See [spec](../branch-switcher.md).

**Blocked by:** 24 (Local branch list read and switch writer (core))

**Status:** ready-for-agent

- [ ] Trigger is the branch line at the top of the details pane for the selected worktree, on every worktree including main and linked
- [ ] Dropdown lists all local branches from the read, alphabetical, searchable, current branch check-marked in place
- [ ] Branches checked out in another worktree are disabled and annotated with that worktree's folder name; they cannot be picked
- [ ] Detached worktree: identity line unchanged, trigger shows a placeholder, picking a branch switches onto it
- [ ] Pick runs the switch in the background; trigger shows "Switching…", is disabled and refuses a second open while running
- [ ] Selection change during a switch doesn't cancel it; the outcome notice still posts
- [ ] Success: "Switched to `<branch>`" confirmation notice; single-repo refresh rereads the repo so the worktree re-nests under the new branch node, badges and recent commits update, snapshot cache saves; selection stays on the same worktree row
- [ ] Failure: status bar shows git's brief reason; worktree state unchanged; no special-case parsing of the refusal
- [ ] A branch created or deleted in a terminal appears/drops from the dropdown after the next repo-level refresh (watcher-driven within 2 s), with no Grove action
- [ ] Keyboard: trigger reachable in focus order, dropdown operable with arrows and Esc; no mouse required
- [ ] VoiceOver: trigger labelled with what it changes, outcome announced via the status bar notice
- [ ] Thin GPUI tests per the spec's Testing Decisions (trigger presence/annotation/disabled matrix, progress state, notices, re-nesting), and confirmation the popover and switcher gained no switch affordance