# 32: Contract — every surface reads the active theme

**What to build:** The old fixed-color helpers and constants are deleted; the widget builders (buttons, choices, switches, radio, selection bar, mono/path text) thread the app context instead. All surfaces now follow the active theme, and the theme module's 4.5:1 contrast matrix is computed from resolved Grove tokens after applying Grove Dark — proving the palette plumbing is true to today's colors. Users still see no change (only Grove Dark is registered), but the engine is live end to end.

**Blocked by:** 29, 30, 31

**Status:** ready-for-agent

- [ ] Raw color literals outside the theme module are gone too — e.g. the tree-view legend folder chip, `.bg(rgb(0x17222f))` in `tree_view.rs` (pre-existing before ticket 29, flagged by the ticket-29 review) — replaced with palette tokens/derivations
- [ ] Fixed-color helpers and their constants are gone; no caller remains outside the theme module's own tests
- [ ] The contrast matrix (every text token on every surface, incl. on-accent pairs) runs on resolved tokens under Grove Dark
- [ ] Tooltips/inputs and other gpui-kit widgets keep their existing contrast guarantees (their gpui-kit test still passes)
- [ ] Zero visual change under Grove Dark
- [ ] `just check` green
