# Two-line recent commits in the details pane

Extends the [details-pane ticket](tickets/07-details-pane.md) and the [resizable-details-pane spec](resizable-details-pane.md). Where they differ, this spec wins. The pane's RECENT COMMITS list stops being a single cramped line per commit and becomes a two-line row: what happened on top, who and when beneath, with a hairline separating rows. Nothing else in the pane changes unless stated.

## Problem Statement

The RECENT COMMITS section squeezes everything about a commit onto one line: hash, subject, and relative time competing for the user-resizable pane's width. Titles are the one thing I actually read, yet they ellipsize first — two of the last five commits in a normal session are already cut off. I want to also see who committed, and there is simply no room: adding the author to the existing line would make the truncation worse, not better.

## Solution

Each commit gets two lines. The subject owns line one, truncating only when it genuinely cannot fit. Line two carries the metadata in the quiet tier: short hash, the author's plain name, and the relative time. A subtle hairline divider between rows keeps each commit's two lines reading as one unit and separates one commit from the next. The list takes roughly twice the height it did; the pane already scrolls and five commits is the established count, so nothing else moves.

## User Stories

1. As a developer, I want each commit's title on its own line, so that long merge and review titles stay readable until the pane truly runs out of width.
2. As a developer, I want the commit's author name shown beside the hash and relative time on the second line, so that I know who landed each recent change without opening a terminal.
3. As a developer, I want the author shown as the plain git author name (e.g. `olsonjj`), so that it reads the way other tools and the log itself do.
4. As a developer, I want the author line to distinguish the author (who wrote it) from the committer (who landed it), so that the name matches what a code review cares about.
5. As a developer, I want the second row to sit in the pane's muted text tier at a size just below the subject, so that the hierarchy reads "headline, then metadata" without effort.
6. As a developer, I want the short hash to keep its link color on the second line, so that the visual anchor identifying a commit stays put.
7. As a developer, I want the relative time on the second line's right edge, so that the column of times remains scannable down the list.
8. As a developer, I want a subtle hairline divider between commit rows, so that one commit's two lines read together and adjacent commits don't blur into each other.
9. As a developer, I do not want a divider after the final commit, so that the list doesn't trail off with a stray rule.
10. As a developer, I want the divider to be a half-step above the panel background, so that it separates without looking like card chrome.
11. As a developer, I want each row to carry inner padding, so that the divider breathes and the tint of two lines-per-commit stays an affordance, not a stripe.
12. As a developer, I want the subject to truncate on its line only, so that missing width costs message text before it ever costs author, hash, or time.
13. As a developer, I want the author to be the piece that truncates on the second line, so that the fixed-width identifiers (hash, time) survive even at the pane's narrowest.
14. As a developer, I want long author names to ellipsize gracefully, so that an extreme name degrades instead of pushing the time off the pane.
15. As a developer, I want the recent-commits row identity (the `commit` element ids) untouched, so that existing tests, keyboard behavior, and screen-reader structure over the list keep working.
16. As a VoiceOver user, I want each commit still exposed as a single unit, so that I hear "hash, author, time" with its subject rather than as disconnected fragments.
17. As a developer, I want the list to still show five commits, newest first, with the same loading/error/empty states, so that the section's behavior (not just its layout) is unchanged.
18. As a developer, I want the two-line rows to reflow with the pane's resizable width (no overlap, re-truncation, no overlap), so that the feature composes with the resizable pane rather than fighting it.
19. As a developer, I want a commit with an empty author (a malformed or anonymized git config) to render with the gap absorbed, so that the row never shows a dangling separator or shifted time.
20. As a developer, I want the change to carry through every reader of the commit read, so that the snapshot/refresh pipeline that feeds this pane is unchanged apart from the extra field.

## Implementation Decisions

- **Layout shape** — a commit row is a vertical stack: full-width truncating subject; beneath it a horizontal metadata line (short hash, author, relative time) that keeps the row's element ids and `test_support` marker on the container, so consumers of the pane see one element per commit as before.
- **Field additions** — the core commit record gains an author field alongside its existing short hash, subject, and committer time. The git log read asks for the author's name with the standard author-name format spec (plain name, not email, not committer), and the parser gains a fourth field. The author parse follows the existing convention: absent simply degrades to empty, it never fails the read.
- **Text hierarchy** — subject keeps the pane body's size and soft text color; metadata drops one size step into the muted/faint tier, hash keeps its link color, time keeps the faint tier. No new font families or weights.
- **Hairline divider between rows** — a one-pixel bottom border on every row except the last, in a new theme color a half-step above the panel background (deliberately well below the card color, per the stripe-vs-card discussion). The row list's inter-row gap is replaced by the rows' inner vertical padding so the divider spacing is symmetric and owned by the row, not by its container.
- **Truncation policy** — subject truncates on line one; on line two the author is the flexible, truncating element between the non-shrinking hash and time. At the pane's narrow clamped width this order degrades the least important piece first.
- **No interaction changes** — commit rows remain non-interactive (no click/hover affordance is added); the section's read path, keying on HEAD, stays as-is, so the author arrives with the same single background read per selection.
- **Theme addition** — one new hairline color function in the theme module, documented as the commit-divider token, alongside the existing worktree-row tint precedent for quiet per-row differentiation.

## Testing Decisions

A good test drives behavior through a public seam and asserts what the user would see — parsed record contents and rendered text — not internal layout machinery or field ordering.

- **Core parse seam (existing)** — the log parser's unit test extends to the new field: a canned log record with the author slot populated and one with it empty parse to records carrying (or defaulting) the author, alongside the existing special-characters assertions. Prior art: the same test's existing hash/subject cases.
- **Core read seam (existing)** — the integration tests over real temp git repos (the details tests are prior art) extend to assert the author: commits made by a known identity come back with that name, and the record's other fields are unchanged. No new seams.
- **App render seam (existing)** — the GPUI test app over a canned repo's assertions extend the existing "five commit rows exist" test to assert the row's rendered text includes the author and the relative time, on the same element ids as today. The layout assertion is behavioral (text present, ids preserved), not geometric.
- **Regression guards (existing)** — the app-level commits test that a linked non-repo worktree shows the read error instead of stale commits runs unchanged.

## Out of Scope

- Clicking a commit (jumping to it in an editor, showing a diff, copying the hash) — rows stay non-interactive.
- Commit counts other than the established five, or any pagination.
- Avatars, email addresses, committer-vs-author display toggles, or date-of-day formatting beyond the existing relative time.
- Alternate row tint (zebra striping) or per-commit cards — the hairline was chosen over both.
- Changes to the loading, error, or empty-commit states, or to any other section of the details pane.
- Light theme; Windows/Linux.