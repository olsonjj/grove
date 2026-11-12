# 28: Cx-aware Grove palette beside the fixed colors

**What to build:** Prefactor for the [theme picker spec](../theme-picker.md). A new palette API on the theme module reads every Grove token (background, panel, sidebar, border, text, muted, accent, …) from the **active theme** (`ActiveTheme`) instead of fixed RGBA constants, with Grove's bespoke dark-tuned tints (worktree row shading, commit divider, connector, on-accent, accent hover/divider, warn pair, text soft/faint variants) derived from a theme's tokens by one documented function. Every existing fixed-color helper still works unchanged, so nothing is visibly different — the codebase is just ready for theming.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] New palette API exposes every token the fixed-color helpers expose, reading the active theme via the app context
- [ ] Bespoke tints derive from theme tokens in one mapping function per the spec's Implementation Decisions (blend/interpolate/darken rules)
- [ ] Unit tests cover the derivation helpers (blend, interpolation, contrast-safe choices) independently of gpui
- [ ] No call site changes yet; old helpers untouched; visuals identical
- [ ] `just check` green
