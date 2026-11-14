# 38: Check for Updates…, end-to-end

**What to build:** The manual check path from the app menu (`specs/updater.md`, "Check state single and global" and "Result dialogs" decisions). The "Check for Updates…" item sits in its slot between About Grove and Settings… (from ticket 37). Triggering it runs one update check on a background thread and reports back over the existing async-channel event path; a single shared check state makes a second trigger while checking a no-op. A dialog shows progress while checking, then one of: "You're up to date" with the running version (manual checks only), "Update available" naming the new version with a Download… button that opens the release page in the browser via NSWorkspace, or check-failed with a Retry button. Repeats are fine: manual checks always report the truth. The fetch override from the engine keeps tests off the network.

**Blocked by:** 36 (core update-check engine), 37 (app menu — About Grove + Settings…).

**Status:** ready-for-agent

- [ ] "Check for Updates…" appears in its reserved slot and triggers the check through the shared engine; triggering again while checking is a no-op
- [ ] Result dialogs for all three outcomes (up-to-date, update-available with Download… opening the release page via NSWorkspace, check-failed with Retry); retry re-runs the check
- [ ] The check runs off the main thread and reports through the existing platform-event path; a slow/dropped fetch never blocks the UI
- [ ] App-level tests with an injected fetch: available shows the dialog naming the new version, up-to-date and failed paths show their dialogs, repeat-trigger ignored while checking (gpui-kit test dispatch)
- [ ] `just check` green
