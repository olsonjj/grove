# 22: "Pull" button in the details pane

**What to build:** a Pull button in the details pane's STATUS section beside the Sync field for the selected worktree when the last read status shows the worktree is openable, on a branch (not detached), tracking an existing upstream, and behind ≥ 1. Clicking runs the pull off the UI thread with the button reading "Pulling…" and disabled (single-flight; a selection change mid-pull doesn't cancel it and its notice still lands). On success a confirmation is posted and that one worktree's status is reread through the existing single-worktree refresh path, so badges, the sync line and recent commits update and the snapshot cache saves. On failure only git's brief reason is shown in the status bar and nothing changes. Stale counts don't block pulling — pull fetches for itself. The button exists nowhere else (absent from popover and switcher). See [spec](../pull.md).

**Blocked by:** 21 (Fast-forward pull in the git writer (core))

**Status:** ready-for-agent

- [x] Button present only when openable + on a branch + upstream tracking + behind ≥ 1; absent on no-upstream, upstream gone, detached, up to date, and not-openable worktrees
- [x] Click runs the pull in the background; button shows "Pulling…", is disabled and refuses further clicks while running
- [x] Selection change during a pull doesn't cancel it; the outcome notice still posts
- [x] Success: confirmation notice posted; the pulled worktree's status is reread so its badge clears and the sync line reads up to date; recent commits re-read on their own because HEAD changed; snapshot cache saves
- [x] Failure: status bar shows git's brief reason; worktree state unchanged
- [x] Stale (failed-fetch) counts don't disable the button
- [x] VoiceOver: button labelled, outcome announced via the status bar notice
- [x] Thin GPUI tests per the spec's Testing Decisions (presence/state/notice matrix), and confirmation the popover and switcher gained no Pull affordance