# Updater: About window, update checks, and app-menu entries

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins.

## Problem Statement

Grove is a hand-distributed app (a zip for testers, manual installs). Once installed, it looks frozen: there's no way to learn what version is running without opening the bundle, and no way to know a newer build exists. The app menu doesn't even offer "About Grove", so the name, version, and project page are invisible from the app itself.

## Solution

1. **About Grove.** The app menu gains a conventional "About Grove" entry that opens a small window showing the app icon, name, version and build, and a link to the project's GitHub page.
2. **Check for Updates…, in the app menu.** A conventional "Check for Updates…" entry runs a check against the GitHub Releases API. The result shows in a small dialog: "You're up to date" with the running version, or "Update available" with the new version and a Download button that opens the releases page in the browser. Failures report quietly and never nag.
3. **Check on launch.** An automatic check runs once at launch, controlled by a general settings toggle (on by default). It stays completely silent unless a version newer than the running one is found *and* the user hasn't already dismissed that version; then it shows the same Update-available dialog as a manual check.
4. **Check results aren't nagged.** The latest version the user has seen (or dismissed) is persisted; the auto-check never repeats an alert for a version the user already saw, while a manual check always reports honestly, even for a dismissed version.

The app menu also gains a conventional **Settings…** entry (⌘,) that opens the existing Settings dialog — a separate gap, but in the same menu.

## Glossary additions

- **Update check** — one query of the releases endpoint for the latest published version, compared against the running version.
- **Running version** — Grove's own version plus build number, the same strings Settings already shows.
- **Latest release** — the newest published GitHub Release, identified by its tag and its release page URL.
- **Dismissed version** — the newest version the user last saw alerting or via the auto-check; persisted, used only to keep the auto-check from repeating itself.
- **Update-available dialog** — the small dialog offering Download…; shown by a manual check when a new version exists, and by the auto-check only for a newer, not-yet-dismissed version.
- **Up-to-date dialog** — the small dialog confirming the running version is current; shown by a manual check only, never by the auto-check.

## User Stories

### About Grove

1. As a developer, I want an "About Grove" item first in the app menu, so that it sits where every other Mac app puts it.
2. As a developer, I want the About window to show the app name, icon, version, and build number, so that I can report my version verbatim when a maintainer asks.
3. As a developer, I want a link to the GitHub project in the About window, so that I can reach the repo without hunting for the URL.
4. As a developer, I want the links (repo, and everything else in these dialogs) to open in my browser via a normal click, so that nothing intercepts or copies the URL.

### Checking for updates

5. As a developer, I want a "Check for Updates…" item in the app menu, so that checking lives where macOS convention puts it rather than buried in Settings.
6. As a developer, I want the check to run against Grove's GitHub releases, so that what it reports matches what's actually published.
7. As a developer, I want to see a progress state while the check runs, so that it doesn't look hung on a slow network.
8. As a developer, I want "You're up to date" with the running version when nothing's new, so that I can move on quickly.
9. As a developer, I want the dialog to name the new version when one exists, so that I know whether the update is worth my time.
10. As a developer, I want a Download… button that opens the GitHub releases page in my browser, so that I can read notes and grab the latest zip.
11. As a developer, I want a failed check (offline, rate-limited, unexpected response) to say so plainly, so that I understand no conclusion was reached and can retry.
12. As a developer, I want a manual check to be repeatable, so that I can look again after a release lands without relaunching.
13. As a VoiceOver user, I want the check-result dialogs announced with their text and button labels, so that I can act on them without the mouse.

### Check on launch

14. As a developer, I want Grove to check for updates once at launch, so that I hear about new releases without remembering to check.
15. As a developer, I want the auto-check silent when I'm current, so that launch isn't interrupted by noise.
16. As a developer, I want the auto-check to surface the update-available dialog when a genuinely new version is out, so that the news reaches me at the same place the app is.
17. As a developer, I want the auto-check to stay silent for a version I've already dismissed, so that one look is enough until something newer ships.
18. As a developer, I want a check-for-updates toggle in Settings (general toggles section), so that launch-time checks are my choice, with the default on.
19. As a developer, I want the auto-check to do one anonymous request and nothing else — no telemetry, no identifiers, so that Grove's network behavior stays as conservative as everything else it does.
20. As a developer on a metered or offline network, I want a failed auto-check to be invisible (no dialog), so that a connectivity blip never surfaces as brokenness.

### Settings and menu plumbing

21. As a developer, I want a conventional Settings… (⌘,) item in the app menu, so that I open Settings the way I open preferences in every other Mac app.
22. As a developer, I want the menu state to stay correct — no crashes, duplicate dialogs, or double checks if I hammer the entries, so that the menu feels native.

### Robustness

23. As a developer, I want version comparison to tolerate tags with or without a leading "v" and patch/build variation, so that release tagging conventions don't break detection.
24. As a developer, I want the running version always to be derived from one place per surface (About, Settings, dialogs), so that no surface can disagree about what's running.
25. As a maintainer releasing a first public build, I want the check to degrade gracefully when zero releases exist yet, so that early builds don't error.

## Implementation Decisions

- **Releases on GitHub.** The check queries GitHub's Releases API for the latest release of the public repo (`olsonjj/grove`) and takes the tag and the release page URL from the response. Download… opens the repo's `releases/latest` page, which redirects to the newest release — the link needs no API call. All URLs derive from the repo; no other infrastructure.
- **Version comparison.** The published tag is parsed with a real semver parser, stripping an optional leading `v`. A release counts as newer only if its semver compare is strictly greater than the running version's. Build numbers never take part in the comparison. Zero releases parse as "no latest release" and yield up-to-date.
- **Networking.** One blocking HTTP getter, pinned `ureq`, standard JSON headers for the API call. It runs on a background thread and reports back over the async channel Grove already uses for platform events. A failed request (offline, non-200, bad JSON) maps into a check-failed outcome rather than an error path — failures are UI states, not crashes.
- **Check engine lives in grove-core.** Fetching, parsing, comparing, and the decision (up-to-date / available / failed / no-releases) live in grove-core as pure logic over one injected fetch, keeping grove-app a thin shell over it. The engine returns the release page URL it got, not a path Grove constructs.
- **Check state single and global.** One app-level check runs at a time: a second trigger during a check is a no-op, and the dialogs reflect the shared state. The auto-check and the manual check share the engine, the state, and the dialogs.
- **Dismissed-version bookkeeping.** The newest version the user has been shown persists in settings.json alongside the toggle. A manual update-available action also counts as seen; dismissed-version data only gates the auto-check — manual checks always report the truth.
- **Menu.** The Grove menu gains, in order: About Grove (first), separator, Check for Updates…, Settings… (⌘,), separator, the existing Hide/Show/Quit block. New gpui actions for all three entries, wired alongside the existing menu actions; Settings… routes to the existing open-settings path.
- **About window.** A small custom gpui window (same patterns as the Settings dialog): icon, "Grove {version} (build {n})", a GitHub repository link, and — for consistency with the dialogs — nothing editable, no draft/Save machinery. Opening it twice focuses it instead of stacking windows.
- **Result dialogs.** Small native-shaped dialogs for the three outcomes (up-to-date, update-available with Download…, check-failed with a Retry). Up-to-date and failed appear from manual checks only; update-available appears from either entry point.
- **Auto-check timing.** One check shortly after launch completes, when the settings toggle is on. Never more than one per launch. The toggle lives with Grove's other general toggles and persists through the normal settings-save path.
- **URL opening.** All link opening goes through NSWorkspace URL opening, the same native surface the app already uses for launching things; no new browser machinery.
- **Version readout stays single-sourced.** The version + build string stays derived from the workspace version and build-number in one place and is reused by every new surface.

## Testing Decisions

- **Good tests** target observable behavior through existing seams, not internals: given a canned releases response, what does Grove report and show; given user actions, what dialog appears and how; given saved settings, what the auto-check does on next launch.
- **The one new seam is the injected fetch.** The check engine takes its HTTP getter as a parameter, and grove-app carries a test override (same Global-override pattern as the pull writer). Tests inject canned success, failure, malformed, and empty-releases JSON — the network is never touched.
- **Core engine tests** are plain unit tests over fixture responses in grove-core: parsing, version comparison (v-tags, equal, older, gaps), and the gating decisions. Prior art: grove-core's existing parser tests.
- **App-level tests** use gpui-kit's test dispatch: trigger Check for Updates… with an injected fetch, assert dialog outcomes; trigger About Grove, assert the window. Prior art: the Settings dialog tests.
- **Settings round-trips** follow the established file round-trip pattern: the toggle and dismissed-version survive save/load and correctly gate an auto-check that would otherwise alert.

## Out of Scope

- **Auto-updating** (downloading, verifying, swapping the bundle, relaunching) — explicitly deferred; the later work, gated on real code-signing/notarization (Sparkle is the leading candidate).
- **Release notes rendering** in Grove — the report dialog carries the new version only; notes live on the release page.
- **Publishing automation** — how releases get drafted/uploaded stays out of Grove's code.
- **Telemetry of any kind**, delta updates, multi-repo/nightly channels, and any non-macOS targets.

## Further Notes

- The releases repo is currently `olsonjj/grove`; if it moves, the check and URLs move with it in one place.
- GitHub's unauthenticated API allows enough requests per hour for launch-time checks; failures from rate limiting surface as the quiet check-failed path.
- The existing "Check for updates" phrasing should stay consistent between the menu entry, dialogs, and the Settings toggle so nothing reads as two different features.
