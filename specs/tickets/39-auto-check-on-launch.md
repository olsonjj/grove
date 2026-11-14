# 39: Auto-check on launch + dismissal

**What to build:** The automatic check and its gating (`specs/updater.md`, "Auto-check timing," "Dismissed-version bookkeeping," and user story 14–20 decisions). A "check for updates on launch" toggle joins Settings' general toggles, default on. With it on, one check runs shortly after launch — never more than one per launch. It is fully silent when the running version is current or the check fails; it shows the update-available dialog only when the latest version is newer than the running one *and* newer than the persisted dismissed version. Showing the update-available result (from either entry point) records the version as seen; the dismissed version persists through the normal settings-save path, lives only in the auto-check's gating, and manual checks ignore it and always report truthfully.

**Blocked by:** 38 (Check for Updates…, end-to-end).

**Status:** ready-for-agent

- [ ] Settings toggle in the general section, default on, persisting through save/round-trip and restored on launch
- [ ] With the toggle on, one auto-check runs per launch after startup settles; off means no check at all
- [ ] Silence: no dialog when up-to-date, check-failed, or the latest version is not newer than either the running or the dismissed version; auto-check shows the update-available dialog only for a newer, not-yet-dismissed version
- [ ] Showing update-available records the version as seen; dismissed-version persists and gates only the auto-check; a manual check still reports an available update honestly
- [ ] Round-trip tests for toggle and dismissed version (settings-file pattern) plus app-level tests of the gating with an injected fetch
- [ ] `just check` green — closes out the updater spec
