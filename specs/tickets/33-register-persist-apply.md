# 33: Register the bundled themes, persist the choice, apply at launch

**What to build:** The first user-visible slice. The ~20 gpui-kit built-in theme JSONs are vendored into the repo with a provenance note (source repo, commit, Apache-2.0) and are registered into the theme registry at startup, embedded so there is no file watcher and no on-disk resources. Settings gains a `theme` choice (a named theme; absent = Grove Dark) persisted through the normal save path, resolved and applied at launch so Grove opens re-skinned. An unknown stored name falls back to Grove Dark without rewriting the file. A debug-only launch hook (env var) opens the app with a canned repo and, optionally, the Settings window, so visual QA can capture deterministic screenshots into `qa-shots/` per theme. Grove Dark's translation and registration as the dark-mode default already landed in ticket 30a (pulled forward after the ticket-30 review found the interim shadcn drift); this ticket builds the choice surface on top of it. Note: gpui-kit 0.8.1 exposes no way to override the registry's built-in dark default, so 30a installs Grove Dark via `Theme::apply_config` rather than an `is_default` flag in the theme file; both routes converge on `Theme::change(Dark, …)`, and 33 must restate the opaque selection token after any re-apply (see the `theme.rs::init` comments).

**Demo:** put another theme's name in a temp settings file, launch with it, and every visible window comes up in that theme.

**Blocked by:** 32

**Status:** ready-for-agent

- [ ] Grove Dark applied at launch by default (translation/registration landed in 30a)
- [ ] All bundled themes registered at startup; launch applies the persisted choice
- [ ] `theme` choice round-trips through the settings file with the other preferences
- [ ] Unknown stored theme name → Grove Dark at launch, stale name left untouched in the file
- [ ] Per-bundled-theme contrast sweep documents (not fails) any derived-token pair below 4.5:1, noted for tuning
- [ ] Debug launch hook renders a canned repo (and Settings window on demand) in a debug build only; captures land in `qa-shots/`
- [ ] Visual check via the harness: main window, details pane and Settings in Grove Dark, one light theme, one dark theme
- [ ] `just check` green
