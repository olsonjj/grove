# 27: Details pane: two-line commit rows with hairline dividers

**What to build:** the details pane's RECENT COMMITS list becomes two lines per commit: the subject alone on line one, truncating only when the pane cannot fit it; beneath it the metadata line in the quiet tier — short hash in its link color, the author's plain name from ticket 26, relative time at the right edge. A hairline one-pixel divider in a new theme token (half a step above the panel background, well below the card color) separates rows, with no divider after the final row; inter-row spacing is owned by the rows' inner padding, not the container's gap. The author is the truncating piece on line two, so hash and time survive at the pane's narrowest. Rows keep today's element ids and unit-ness for tests, keyboard, and VoiceOver; the section's read path, count, and loading/error/empty states are untouched. See [spec](../commit-details-two-line.md).

**Blocked by:** 26 (Core: commit record and log read carry the author name)

**Status:** ready-for-agent

- [ ] Each commit renders as a vertical pair: full-width truncating subject, then hash · author · relative time beneath in the muted/faint tier at one size step below the subject
- [ ] The short hash keeps its link color on the metadata line; the relative time sits at the metadata line's right edge
- [ ] The author takes the flexible, truncating slot on the metadata line between the non-shrinking hash and time; a long author ellipsizes before pushing the time off the pane
- [ ] A commit with an empty author renders with the gap absorbed — no dangling separator or shifted time
- [ ] Hairline dividers sit between rows (none after the last), in the new theme token; row spacing comes from inner padding, not the container gap
- [ ] Existing commit-row element ids and per-commit unit structure are preserved, so the app-level five-rows and read-error tests pass unchanged
- [ ] Thin GPUI render tests per the spec's Testing Decisions: rendered row text includes the author and relative time on the preserved ids; regression guards for the five-rows matrix and the linked non-repo worktree error