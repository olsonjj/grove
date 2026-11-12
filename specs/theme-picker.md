# Theme picker in Settings

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins.

## Problem Statement

Grove is locked to one hand-tuned dark palette (`crates/grove-app/src/theme.rs` holds fixed RGBA constants; `Theme::change(ThemeMode::Dark, …)` only colors gpui-kit's own widgets). There is no way for the user to change how Grove looks, and nothing to build on if we ever want a light mode. gpui-kit ships ~20 built-in themes (light and dark) that Grove doesn't use.

## Solution

1. **Retheming engine.** Grove's own surfaces stop reading fixed colors and start reading the active gpui-kit theme (`cx.theme()`, `ActiveTheme`). The current Grove palette becomes "Grove Dark", a registered theme like any other, so it is the default and also a picker choice. Applying a theme re-skins every visible window live (main window, menu bar popover, quick switcher, settings).
2. **Theme choice, saved like everything else.** The Settings dialog gains a Theme dropdown with search. Picking applies immediately so the user sees what they get; it stays in the window's draft until **Save**. Cancel / Escape reverts to the theme that was active when the window opened. Save persists the choice, and it is restored on every launch.
3. **Follow System.** The picker also offers "Follow System", which tracks macOS's appearance live: dark → Grove Dark, light → Default Light. Turning the Mac to Light Mode with Follow System chosen switches Grove immediately, like a picked theme.

The palette lives in gpui-kits's `ThemeConfig` (JSON), which also ships highlight (syntax) colors — so code colors in the details pane follow the theme too.

## Glossary additions

- **Theme** — a gpui-kit `ThemeConfig` in the `ThemeRegistry` (a name plus color tokens, light or dark mode, including highlight styles). The bundled set has Grove Dark plus the ~20 gpui-kit built-ins.
- **Theme choice** — what the setting stores: `Follow System`, or a Theme by name. `Grove Dark` is the default Theme choice.
- **Resolved theme** — the Theme actually active: for `Follow System`, the mode default (dark → Grove Dark, light → Default Light); for a named Theme, that Theme.
- **Draft theme choice** — the choice picked in the open Settings window. Applied live, never persisted until Save.
- **Entry theme** — the resolved theme active when the Settings window opened; what Cancel/Escape reverts to.

## User Stories

### Choosing a theme

1. As a developer, I want a Theme section in Settings with a dropdown showing my current theme, so that I can find the setting in one glance.
2. As a developer, I want the dropdown to open a searchable list of themes, so that I can type "gra" and land on Gruvbox instead of scanning ~20 rows.
3. As a developer, I want the list grouped in SYSTEM (Follow System), LIGHT and DARK sections, so that I can tell which themes suit my mood before trying them.
4. As a developer, I want the active choice marked with a check in the list, so that I know what is applied right now.
5. As a developer, I want arrow keys and ⏎ to work in the list, so that I can try themes without the mouse.
6. As a developer, I want picking a theme to re-skin Grove in real time across all visible windows, so that the choice is visible before I commit.
7. As a developer, I want the settings window itself re-themed too, so that I'm previewing with the same chrome as the rest.
8. As a developer, I want Escape or Cancel to revert to the entry theme, so that experimenting is free.
9. As a developer, I want Save to persist my choice so Grove opens with it next launch, consistent with every other setting.
10. As a developer, I want my syntax-highlight colors in the details pane to follow the theme, so that code stays readable instead of clashing.

### Follow System

11. As a developer, I want a "Follow System" choice, so that Grove matches macOS's appearance without me re-picking.
12. As a developer, I want Follow System to resolve dark → Grove Dark and light → Default Light, so that both modes have a designed, default look.
13. As a developer, I want Grove to retheme live when macOS's appearance changes while Follow System is chosen, even with Settings open, so that it truly follows.
14. As a VoiceOver user, I want the picker announced with its label, current value, and each row as "<theme name>, <light|dark>, selected", so that I can choose by ear.

### Robustness

15. As a developer upgrading Grove, I want an unknown stored theme name (removed/renamed) to fall back to Grove Dark, so that the app never fails to launch or shows a broken mix.
16. As a developer, I want the entry theme reverted even if I changed my mind several times while the window was open, so that Cancel always means "as it was when I opened Settings".

## Implementation Decisions

### Retheming engine (the big piece)

- **`theme.rs` reads `ActiveTheme`.** Every color helper (`bg()`, `panel()`, `muted()`, … — ~334 call sites) gains a `cx: &App` (or `&Window, &mut App` where natural) parameter and returns the projected value from `cx.theme()` instead of a constant. `theme::init` stays the startup hook (called from `main.rs`). Widget builders (`button()`, `mono()`, …) pass `cx` through. Mechanical, wide, no behavior change when Grove Dark is active.
- **Grove tokens are a mapping over theme tokens.** Where a theme provides the token directly, use it (background → `bg`, panel → `panel`, popover/card → `card`/`raised`, accent → `accent`, border → `border`, foreground → `text`, muted → `muted`, …). The bespoke dark-tuned tokens have fixed derivations from the theme's tokens:
  - `worktree_row` — background blended toward the accent (same relative step as today's `#151d27` is above `#121418`).
  - `commit_divider` — linear interpolation between `panel` and `card`, halfway.
  - `connector`, `faint`, `text_soft` — lightness/darkness variants of `border` / `text` respectively.
  - `on_accent`, `on_accent_divider`, `accent_hover` — accent-derived (darkened/inverted) so accent buttons stay legible in light themes.
  - `warn_bg` / `warn_text` — derived from the theme's danger tokens, falling back to an orange mix of accent when the theme doesn't define one.
  These derivations are one function `grove_tokens(cx) -> GrovePalette`, unit-testable, so tuning happens in one place, not at call sites.
- **Grove Dark is a registered theme.** Today's palette is translated once into a `Grove Dark` `ThemeConfig` JSON (shipped alongside the built-ins), with `is_default` set for the dark mode, so the default code path and a user's pick take exactly the same route. The 4.5:1 contrast test in `theme.rs` then applies to resolved Grove tokens under Grove Dark — a true regression guard.
- **One code path, light included.** There is no dark-only special case; light themes flow through the same mapping. Known risk: some derivations may look imperfect in odd palettes. Accepted; QA pass to tune, see Testing.
- **Apply = `Theme::update` + registry swap.** To activate a Theme by name: set `Theme::dark_theme`/`light_theme` from `ThemeRegistry::themes()` and `Theme::change(mode, …)` (the registry observer's own pattern). Every window refreshes (gpui-kit's `Theme::update` calls `cx.refresh_windows()`).

### Bundling themes

- The ~20 gpui-kit theme JSONs (from `github.com/longbridge/gpui-kit/tree/main/themes`, Apache-2.0, noted in Further Notes) plus `grove-dark.json` are vendored into the repo under `crates/grove-app/resources/themes/`.
- They are **embedded with `include_str!`** (a `const THEMES: &[(&str, &str)]` of name → JSON), parsed at `theme::init`, and registered into `ThemeRegistry` — no `watch_dir`, no file watcher, no on-disk resources in the .app bundle, no path resolution problems.
- The picker lists whatever is registered at runtime (registry `sorted_themes()` ordering: light before dark, case-insensitive by name), so the list stays in sync with the bundle automatically. Follow System is a fixed first entry.

### Follow System

- Persistence distinguishes choice kinds: `theme: Option<ThemeChoice>` in `Settings`/`Preferences` (grove-core), `ThemeChoice::FollowSystem` or `ThemeChoice::Name(String)`; `None` (absent in old settings files) = `Name("Grove Dark")`.
- Resolution: `FollowSystem` → resolve on every appearance evaluation using `Window::appearance()` (per window) / `cx.window_appearance()`. Mode → default theme: dark = Grove Dark, light = Default Light (gpui-component's default light, as on the gpui-kit site picker).
- Live tracking: each created window registers gpui's appearance-changed callback; on change, if the persisted choice is Follow System, re-resolve and apply. Appearance changes fire per-window; apply at most once per change (the first notification wins; subsequent windows' callbacks see the theme already applied).

### Settings integration (draft → live-apply → revert)

- `SettingsWindow` captures the **entry theme choice** (the persisted `ThemeChoice`, not its resolution, so Cancel during a Follow-System appearance change doesn't pin a resolved name) at `new()`.
- Picking in the dropdown: updates `self.draft.theme`, applies the resolved theme app-wide, `cx.notify()`. Unlike other prefs this leaves the draft; the settings window re-render takes the theme live everywhere.
- `save()` persists `draft.theme` with the rest of `Preferences` (same `save_preferences` path — no special casing).
- Cancel / Escape / window removal by any path: re-apply the entry theme choice. Done in the same place the window closes, so no revert path is missed.
- While the custom-command form is open, the picker stays usable; retheming doesn't disturb the form's state (colors only).
- The Theme section sits under GENERAL (own section header "THEME", matching the section styling), above the version note, inside the existing scrollable body — the window needs no resize.

### The picker

- Reuse grove's section/label language (`label("THEME")`) with a single button row: `theme-picker` button showing the current choice's name + a chevron, styled like today's secondary buttons, with an accessibility label like the other controls ("Theme, <choice>, opens theme list").
- The popover list is gpui-kit's searchable list (`SearchableGroup`/`SearchableListDelegate` — the `select.rs` machinery), matching the site's picker: search input ("Search themes…"), grouped rows (System / Light / Dark), section headers, current choice checked. Keyboard: ⌃↑/↓ or ↑/↓ move, ⏎ picks, Esc closes without picking. Escape first closes the list; a second Escape still cancels the dialog (list consumes it while open).
- A11y labels per story 14. Search filters case-insensitively on theme name ("mac" → macOS Classic Light/Dark).

### Persistence & loading

- `shell::save_preferences` stores the setting like every other pref (same file, atomic replace).
- `theme::init` at launch: register themes, read the saved choice, verify it resolves (story 15: unknown name or empty registry entry → log + fall back to Grove Dark, rewrite nothing — the stale name stays until the user saves a different choice), resolve + apply. This replaces the current unconditional `Theme::change(ThemeMode::Dark, …)`.

## Testing Decisions

- **Contrast regression.** The existing 4.5:1 matrix keeps passing with Grove Dark, now computed from resolved Grove tokens after applying the `grove-dark.json` config — proving the translation is true to today's palette. A companion test applies **every** bundled theme and asserts the derived Grove tokens keep text ≥ 4.5:1 on every derived surface, documenting (and catching) any light-theme derivation that falls below; where a bundled theme is under the bar, the test pins it as a known exception with a `// FIXME(contrast)` note rather than failing forever.
- **Draft semantics** (settings_window.rs test style): picking applies the theme immediately (assert resolved theme name after pick); Cancel reverts to the entry theme; Save persists (`file.load()` round-trips the choice) and the choice survives a re-open. Pick → Cancel → reopen shows the old choice still stored.
- **Follow System** test: simulate appearance change → theme re-resolves; with a named theme, appearance changes do nothing.
- **Fallback** test: stored choice names a theme that isn't registered → resolves to Grove Dark at launch.
- **Mapping unit tests** for `grove_tokens` derivations (blend/interpolation helpers) independent of gpui.
- Manual QA (`qa-shots/`): screenshot the main window, details pane, popover, switcher and settings under Grove Dark, one light theme and one unusual dark theme (e.g. Gruvbox), plus Follow System toggling to Light Mode; verify highlight colors in the details pane and every button/kind (primary/secondary/disabled), radios, switches, inputs, tooltips.

## Out of Scope

- Custom themes or user theme files on disk (no `watch_dir`, no themes folder in the app bundle).
- Per-window themes; theme is app-wide.
- Any other settings dialog theme-related prefs (font size, accent overrides).
- A dedicated "Grove Light" designed palette (Follow System's light mode uses Default Light; a designed Grove Light can land later as just another registered theme).
- Retiring/rewriting `SettingsWindow`'s other sections.

## Further Notes

- The bundled JSONs are copied from github.com/longbridge/gpui-kit `themes/` (unchanged at the commit vendored). gpui-kit licenses docs prose CC BY 4.0; the theme files are part of its Apache-2.0 code, so vendoring with a provenance note (source URL, commit, license) is fine. Add `crates/grove-app/resources/themes/PROVENANCE.md`.
- Theme count is "~20" files but more registered names: several files contain variant sets (one `catppuccin.json` = Latte/Frappuccino/Macchiato/Mocha; `ayu.json` = Light/Mirage/Dark; `tokyonight.json` = Night/Storm/Day…). The picker list is whatever `ThemeRegistry` holds (~30 names); that's intentional — the search field exists for exactly this.
- gpui-component exposes the pieces this relies on (theme 0.8.1): `ThemeRegistry` (global map of `ThemeConfig`s), `Theme::change/update` (refresh every window), `Theme::sync_system_appearance`, `Window::appearance()`. No gpui-kit changes needed.
- If a future gpui-kit upgrade adds themes, bumping the vendored JSONs is a copy + provenance note; the picker needs no change.
