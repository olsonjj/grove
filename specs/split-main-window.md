# Split the main window module

A pure-refactor spec: no behavior changes, no test changes in intent, no new features. After it lands, `main_window.rs` (5,632 lines) and `main_window/mod.rs` (~900 lines) are the smallest they can get while every test still passes against the same public seams it ran against before.

Extends the module-layout agreement in [Grove v1 spec](grove-v1.md) (the GPUI shell is a thin layer over `grove-core`). It changes no seams — every seam named below already exists in `crates/grove-app/src/main_window.rs`, and the refactor's contract is that they keep behaving identically.

## Problem Statement

`main_window.rs` holds the entire main window: its model structs, constructor and subscriptions, the removal flow, the sidebar (selection, pinning, context menu, manage-visibility popover), the render tree with the top bar and status bar, the resizable pane, and — by far the largest share — roughly 4,000 lines of tests spanning every feature area of the app. The file clusters in a healthy 400–1,600 line band elsewhere in the crate; at 5,632 lines this one module carries ~3.4× the next largest and mixes five concerns, which makes every PR into it a merge hazard and every new feature question a scroll hunt.

## Solution

Turn the flat file into a directory module in the same style the rest of the crate already uses: a `main_window/` directory next to a slim parent, with one child module per concern. The module is internal to the `grove-app` crate — nothing outside `main_window.rs` (`main.rs`, `switcher.rs`, `shell.rs`, …) sees any change beyond the `use` paths of the two items it exports today (`Status`, `main_window::open`). Git history stays reviewable: the split moves code verbatim; it adds no logic, deletes no tests, and renames nothing the user can see.

## Target shape

```
crates/grove-app/src/
  main_window.rs          # slim parent: docs, re-exports, `use` of children, shared constants
  main_window/
    window.rs             # window lifecycle: MIN_SIZE, open(), open_with()
    app_model.rs          # event glue: entity construction, subscriptions (sync/fetch/notice)
    layout.rs             # Render impl, top bar, status bar, pane constants (panes, sidebar)
    sidebar/
      mod.rs              # sidebar orchestration: selection, keyboard nav, pinning, repo menu
      rows.rs             # repo_row + row element helpers (group_header, pin_button, top_button, link, tilde)
      visibility.rs       # manage-visibility popover (button, chip, rows, popover)
    removal_flow.rs       # Target/Removal structs + confirm/cancel/run/finish + removal_footer
  ...
```

The parent keeps `MainWindow` (struct + `impl MainWindow` glue) and re-exports `Status` and `open` so external call sites are untouched. Children pull the pieces they own via `use super::…`.

## Implementation Decisions

- **Verbatim moves.** Each child receives its code exactly as it stands — function bodies, constants, and doc comments unchanged. The only edits are `use` statements and visibility (items shared across children become `pub(super)` or `pub(crate)`).
- **`Status` stays the crate's public item**, re-exported from the parent so `switcher.rs` continues to `use crate::main_window::Status` untouched. Same for `main_window::open`.
- **No new helpers, no inlining, no "while I'm here" fixes.** Anything that looks like a bug found during the move is recorded in the PR body, not fixed here.
- **Tests move into feature-scoped siblings.** The current single `mod tests` (~4,000 lines) divides into `main_window/tests/` with one child per feature area mirroring the ticket list (launch & editors, details pane, sidebar rows & pinning, visibility, resizable pane, removal, pull, branch switcher, badges & status bar). Shared harness helpers (launch, click, found, git, fixtures) live in `main_window/tests/common.rs` and are `pub(crate)`; individual test fns keep their names so failure messages and prior-art references in the specs keep resolving.
- **Ordering** for review: (1) skeleton + verbatim non-test moves per child, (2) test harness extraction, (3) test moves per feature area, (4) full check. Each step compiles green before the next starts.
- **The module's `missing_docs` lint** (workspace-wide deny on `grove-app` too) applies to any `pub`/`pub(super)` item introduced by the split — each new child and shared item carries the moved doc comment or a one-line one.

## Testing Decisions

The refactor must not change what any seam asserts.

1. **The suite is the spec.** `just check` (fmt, clippy `-D warnings`, `cargo test --all-features --locked`, doc build) must pass before and after. Test count is asserted in the run log before the refactor starts and must match after the moves; no test is edited beyond its `use` lines and module path.
2. **Same seams, new host.** Tests keep running through the public seams they use today (the GPUI test app over canned snapshots, the settings file, core's `repo_order`) — the moving applies to where the test code lives, not what it drives.
3. **Behavioral guardrails already in place carry the risk**: removal flow, pull/switch exclusivity, resizable-pane clamp/persist, visibility chip/persistence, sort menus. If any of these so much as wobbles, the move is wrong — the fix is to fix the move, not the test.

## Validation

- `just check` clean at every committed step.
- The debug app runs on this machine (`just run <temp settings>`), lists real repos, opens, hides, and closes without unexpected log output — a 30-second smoke pass, screenshot dropped in the PR description.
- `cargo clippy` on both crates stays warning-free (no new dead-code warnings from over-widened visibility).

## Out of Scope

- Any change to `grove-core`, any other `grove-app` module, or any test's assertions.
- Making `grove-app` a lib crate / adding integration tests against it.
- Splitting other large-but-in-band files (`settings_window.rs`, `details_view.rs`).
- Cross-platform extraction of `macos.rs`.
- Renaming moved items, changing visibility beyond what the split requires, or deduplication.