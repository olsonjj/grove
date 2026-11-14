# 32: Contract — every surface reads the active theme

**What to build:** The old fixed-color helpers and constants are deleted; the widget builders (buttons, choices, switches, radio, selection bar, mono/path text) thread the app context instead — and resolve popover's `text_button` redundancy noted by the ticket-31 review (it takes both `color: Hsla` and `cx`; take a `&Palette` or derive the color from a role). All surfaces now follow the active theme, and the theme module's 4.5:1 contrast matrix is computed from resolved Grove tokens after applying Grove Dark — proving the palette plumbing is true to today's colors. Users still see no change (only Grove Dark is registered), but the engine is live end to end.

**Blocked by:** 29, 30, 31

**Status:** ready-for-agent

- [x] Raw color literals outside the theme module are gone too — e.g. the tree-view legend folder chip, `.bg(rgb(0x17222f))` in `tree_view.rs` (flagged by the ticket-29 review), and `rgb(0x2c4560)` in `setup.rs` (flagged by tickets 30/31 reviews) — replaced with palette tokens/derivations
- [x] Fixed-color helpers and their constants are gone; no caller remains outside the theme module's own tests (25 deleted; `selected()` kept `pub(crate)` for init's selection-clamp restate; colors were inline literals)
- [x] The contrast matrix (every text token on every surface, incl. on-accent pairs) runs on resolved tokens under Grove Dark (real init path; legend chip included as a surface)
- [x] Tooltips/inputs and other gpui-kit widgets keep their existing contrast guarantees (their gpui-kit test still passes under real init)
- [x] Zero visual change under Grove Dark (reviewer-adjudicated sub-JND deltas only: setup folder-field border collapsed to border_strong, oklab ΔE 0.030 equal to the old two-state difference; legend chip derived bg→link at LEGEND_CHIP_TINT 0.08, dL 0.002 from the old literal)
- [x] `just check` green (full `cargo test --all-features --locked`; only the pre-existing unrelated `grove-core::freshness` flake fails)
