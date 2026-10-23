# QA checklist — two-line recent commits (tickets 26 & 27)

Branch: `feature/commit-details-two-line` (pushed; single PR for both tickets).
App build for QA: `target/release/bundle/osx/Grove.app` (build 15). Rebuild fresh with `just bundle` if desired.

A scratch fixture for this QA lives at `/tmp/grove-qa/`: `code/demo` (4 commits, author `olsonjj`) and `code2/alpha` (2 commits), with matching settings files (`settings.json`, `settings2.json`). Launch with:
`GROVE_SETTINGS_PATH=/tmp/grove-qa/settings.json target/release/bundle/osx/Grove.app/Contents/MacOS/Grove` (quit any running Grove first).

## Core read (ticket 26) — covered by automated tests, spot-check on screen

- [ ] RECENT COMMITS shows five commits, newest first (make ≥ 6 in a repo to confirm the cap)
- [ ] Second line shows the **author's plain name** (e.g. `olsonjj`) — verify a repo where the author differs from the committer (e.g. after applying an emailed patch) shows the author, not the committer
- [ ] Subject, hash, time are unchanged from before; only the layout and author are new

## Two-line rows (ticket 27) — visual/interaction

- [ ] Each commit is a vertical pair: subject on line one, `hash · author · time` beneath
- [ ] Subject keeps the pane's body text size/color; metadata line is one size step smaller and quieter (hash stays link-blue in mono, time faint at the right edge)
- [ ] A hairline divider separates adjacent commit rows; there is **no** divider after the final (bottom) commit
- [ ] Divider is subtle: reads as separation, not card chrome (compare against the STATUS section's card-free look)
- [ ] Rows are evenly spaced; spacing is owned by the rows, not the section container

## Truncation & reflow with the resizable pane

- [ ] Drag the pane narrow: subject truncates on its own line first
- [ ] At the narrowest clamped width, the short hash and the time still fit; the **author** is what ellipsizes
- [ ] A very long subject stays on one line (no wrap pushing the row taller) and ellipsizes
- [ ] A very long author name ellipsizes with `…` instead of pushing the time off the pane
- [ ] Drag the pane wide again: everything re-truncates back, nothing overlaps

## Empty-author degenerate case

- [ ] A repo with a malformed/anonymized git author (if you can produce one) shows the gap absorbed: hash, then time at the right edge — no dangling separator, no shifted time
  (note: git refuses empty `user.name`, so this is easiest to produce by editing a commit's author to spaces; the parse path is unit-tested)

## Behavior unchanged

- [ ] Rows remain non-interactive: no click, hover highlight, or cursor change on a commit row
- [ ] Loading ("Reading…"), error (non-repo linked worktree), and empty ("No commits yet") states render as before
- [ ] Screen reader (VoiceOver): each commit reads as one unit — hash, author, time, subject — not disconnected fragments
- [ ] Recent-commits still re-read on branch switch and pull (metadata line updates too)