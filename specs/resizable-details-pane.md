# Resizable details pane

Extends [Grove v1 spec](grove-v1.md). Where they differ, this spec wins. The details pane stops being a fixed-width column and becomes user-resizable; nothing else about the three-column layout (repo list sidebar, branch tree) changes unless stated.

## Problem Statement

The details pane is a hard-coded 320 px column. On a wide window it wastes space while long content inside it truncates — worktree paths, changed-file paths, commit messages — and I end up opening a terminal to read what's actually there. Meanwhile Grove is wide enough that the pane could be twice as wide. On a narrow window I have the opposite problem: the fixed pane squeezes the branch tree when I'd rather give the tree the room.

## Solution

A draggable divider between the branch tree and the details pane. Drag left to widen the pane, right to shrink it, within sensible clamps so neither column becomes unusable. The chosen width sticks — across re-renders, repo switches, and app relaunches. Everything else stays where it is.

## User Stories

1. As a developer, I want to drag the divider between the branch tree and the details pane, so that I can trade width between the two columns to fit what I'm looking at.
2. As a developer, I want the details pane to start at its current default width (320 px) on a fresh install, so that the layout I know doesn't change until I change it.
3. As a developer, I want the divider to clamp the details pane to a minimum width, so that I can't shrink it into an unreadable sliver.
4. As a developer, I want the divider to clamp the details pane to a maximum width, so that a careless drag doesn't crush the branch tree.
5. As a developer, I want the branch tree to keep a usable minimum width even when the details pane is at its maximum, so that the tree never collapses.
6. As a developer, I want the divider to look like a divider — a hairline that answers the pointer (hover/press/drag indicator) — so that I can find and trust it.
7. As a developer, I want the divider to cursor as a horizontal resize affordance, so that I know it's draggable before I try.
8. As a developer, I want the pane's content to reflow as its width changes, so that widening it actually reveals more of the truncated text.
9. As a developer, I want the details pane's width to survive selecting a different repo or branch, so that rerendering doesn't snap my width back to the default.
10. As a developer, I want the details pane's width to survive quitting and relaunching the app, so that I set it once.
11. As a developer, I want my resize written when the drag ends (not on every frame), so that dragging never writes I/O at drag rate.
12. As a developer, I want dragging the divider to stay smooth (no git work, no re-scan, no layout thrash beyond the columns), so that resizing a 13 GB repo's window feels free.
13. As a developer, I want the repo-list sidebar to stay at its fixed width while I drag, so that the feature is bounded to the pane I asked about.
14. As a developer, I want the empty state (no repo selected / no repos) to lay out the same as today, so that pre-selection screens are untouched.
15. As a developer, I want the details pane's buttons (open in editor/terminal, copy path, pull) to render correctly at every width in the clamp range, so that nothing overflows or overlaps after a resize.
16. As a developer, I want the open-target buttons, path and status values to truncate gracefully at the minimum width, so that a narrow pane degrades instead of breaking.
17. As a developer, I want window resizing to keep both columns sensible (the persisted pane width respected while the tree flexes), so that resize of the whole window doesn't fight the pane setting.
18. As a VoiceOver user, I want the divider not to trap focus or introduce unlabeled hit targets, so that keyboard navigation of the app is unaffected.

## Implementation Decisions

- **Use GPUI Kit's built-in resizable panels** — the component layer's horizontal resizable panel group with a shared panel-state entity (same pin as the rest of the stack, 0.7.0; no hand-rolled drag logic, no new dependency). Panel state is created with the main window and seeded with the stored width.
- **Scope of the group** — a two-panel group wraps only the branch tree and the details pane, replacing the plain flex row that holds them today. The first panel (branch tree) flexes; the second (details) carries the stored width. The repo-list sidebar stays a fixed-width child outside the group.
- **Clamps** — details pane: 260 px minimum (today's min), 720 px maximum, 320 px default. Branch tree: clamped so a tree ≥ its usable minimum remains when the pane is at maximum. The empty-states branch (no repos) stays a plain layout — the group mounts only when it mounts today.
- **The pane stops owning its width** — the details view's own fixed width/flex-shrink/min-width styling is removed; it renders full-width inside its panel and lets the panel drive sizing. The pane's left border goes away; the panel group owns the divider line between the two columns.
- **Handle appearance** — this is what the Kit panels give for free: a resting hairline with a hover/press/drag pill indicator and a resize cursor. Grove's own theme tokens already feed this path; no custom handle renderer.
- **Persistence** — one new optional field on the shared Settings struct: the details pane's width in pixels, absent on fresh installs (serde skips it, so old settings files load unchanged). It is written through the existing settings file when the drag ends, and seeded into the panel state when the main window builds. Programmatic and drag resizes both end in the same event, so persistence listens once.
- **No git / snapshot interaction** — resizing touches only the window's layout state and the settings file. It never triggers a scan, refresh or fetch, and holds no locks.
- **Window resize** — the tree panel keeps its flex so the window's own resize distributes extra space to the tree first; the pane's stored width is a basis, not an absolute claim.

## Testing Decisions

A good test drives behavior through a public seam and asserts what the user would see — element bounds and rendered structure, not internal drag machinery or which GPUI element the handle is.

- **One seam: the existing thin UI seam.** The GPUI test app over a canned Snapshot, exactly like the main-window tests that already assert layout via element bounds (the open-target placement test is the prior art). No new seams.
  - Fresh launch: the pane's left edge sits 320 px worth from the tree/pane divider, matching today's default; the repo sidebar's width and position are unchanged.
  - Resize: driving the shared panel state (its public programmatic resize, the same path a drag's end uses) moves the pane's bounds and the tree's bounds as expected — the tree shrinks by what the pane gains.
  - Clamps: resizing the pane below its minimum or above its maximum lands on the clamp, and the tree never goes below its own minimum.
  - Width survives selection change: choosing another repo/branch after a resize does not reset the pane's bounds.
  - Persistence: a resize ends with the width in the settings file (temp-dir settings file, as the settings-window tests do), and a relaunch with that file restores the width.
  - The existing layout assertions (editor/terminal buttons within the rows, terminal left of the details pane) still hold at default width — they run unchanged as regression guards.
- **Core settings** covered by the settings file's existing round-trip tests, extended with the new optional field (present/absent).

## Out of Scope

- Resizing the repo-list sidebar.
- Collapsing or hiding the details pane entirely, or a double-click-to-reset on the divider.
- Keyboard resizing of the divider (Grove v1's keyboard story covers actions in the tree and switcher; pointer-only pane resize matches how the columns otherwise work).
- Remembering the window size/position or the tree's width independently of the pane.
- Resizable splits in the settings window or any other window.
- Light theme; Windows/Linux.

## Further Notes

- The panel state's defaults drive the pane's bounds, so tests that assert absolute positions now implicitly depend on the seeded state; seeding explicitly in tests keeps them robust to the Kit's defaults.
- The Kit's resize handle draws the drag pill over neighboring content via deferred painting; a popover opened from a branch row must still paint above the divider — existing popover layering covers this, worth an eye during review.