# Hide repos from the list

Extends [Grove v1 spec](grove-v1.md) and [Sort and pin](sort-and-pin.md). Where they differ, this spec wins. The sort-and-pin decision "the REPOSITORIES · N header keeps counting every repo" (its story 8) is superseded: the header counts the repos the list actually shows, with hidden repos revealed by a chip.

## Glossary additions

- **Hidden repo** — a repo Grove still scans, fetches and remembers, but that the desktop app's sidebar repo list doesn't show: it leaves the REPOSITORIES list and the PINNED group. The menu-bar popover and the quick switcher are untouched by hiding. Hiding is a display choice keyed to the repo's path; it is not archiving, deletion, or removal from search.

## Problem Statement

The sidebar lists every repo Grove finds under my code folder, and I have whole folders of finished projects I haven't touched in months. Pins solve the opposite problem (repos I use constantly), but the long tail — archived experiments, one-off client work, clones I keep for reference — just clutters the list and pushes what I care about further down the scroll.

## Solution

A "manage visibility" control in the sidebar header, next to the sort menu, opens a popover listing every repo Grove found, each row with a show/hide toggle. I sweep through it once, mute the dead weight, and close it. Hidden rows are dimmed and struck through in the popover so my choices stay visible; the header count drops to what's shown and a "· N hidden" chip appears, which reopens the same popover when I change my mind. Hiding applies instantly to the desktop app's sidebar, is remembered by the repo's path across restarts, and touches nothing else — the menu-bar popover (whose search already tames a long list) and the quick switcher keep listing every repo, so a repo I hid is still one keystroke away.

Chosen from a three-variant prototype (right-click menu + hidden expander / manage popover / auto "Older" section); the manage popover won.

## User Stories

### Hiding and unhiding

1. As a developer, I want to hide a repo I no longer work on, so that finished and archived projects stop cluttering my sidebar.
2. As a developer, I want a visibility-manage button in the sidebar header next to the sort menu, so that list curation lives where the list is shaped.
3. As a developer, I want the manage button to open a popover listing every repo Grove found, so that I can curate the whole list in one sitting.
4. As a developer, I want an eye toggle on each popover row, so that hiding or restoring a repo is one click.
5. As a developer, I want hidden repos shown dimmed and struck through at their place in the popover's list, so that I can see what I've already hidden and what a toggle will do.
6. As a developer, I want a hidden row's toggle to bring it back, so that hiding is easy to undo.
7. As a developer, I want my hiding to apply the moment I toggle, with the sidebar behind the popover updating at once, so that I can watch the list shrink as I work.
8. As a developer, I want to close the popover with Escape, an outside click, or its button, so that it never traps me.

### What hidden means on every surface

9. As a developer, I want a hidden repo to disappear from the sidebar's REPOSITORIES list, so that my list only shows repos I might actually use.
10. As a developer, I want a hidden repo to leave the PINNED group too, so that "pinned but nowhere" can't happen; its pin comes back with the repo if I unhide it.
11. As a developer, I want the menu-bar popover to keep listing every repo regardless of hiding, so that the popover stays a complete quick-launch surface that search already tames.
12. As a developer, I want a hidden repo still findable in the quick switcher, so that hiding means "out of my main window's browsing" — not "deleted from my memory".
13. As a developer, I want hidden repos still scanned, fetched, and counted in the worktree trees I open, so that hiding changes nothing about Grove's knowledge of my machine.
14. As a developer, I want hiding one repo not to reshuffle or re-sort the rest, so that my muscle memory of the list survives every toggle.

### Header count and the chip

15. As a developer, I want the REPOSITORIES header to count only the repos shown, so that the number matches the list I'm looking at.
16. As a developer, I want a "· N hidden" chip next to the count whenever anything is hidden, so that I never wonder where repos went.
17. As a developer, I want the chip to open the same manage popover, so that restoring something is as quick as hiding it was.
18. As a developer, I want the chip absent when nothing is hidden, so that the header looks exactly as it does today until I use the feature.

### Remembering

19. As a developer, I want hidden repos remembered across restarts, so that I curate once.
20. As a developer, I want hiding keyed to the repo's location on disk rather than its display name, so that renamed or same-named folders can't confuse it.
21. As a developer, I want a hidden repo to stay hidden when its folder temporarily disappears and returns, so that a volume unmount doesn't resurrect clutter.
22. As a developer, I want a pinned repo's pin preserved while I hide it, so that unhiding restores the repo exactly as I had it.
23. As a developer, I want the Settings window's Save to leave my hidden set untouched, so that editing preferences never clobbers my curation.

### Managing a long list

24. As a developer, I want the manage popover to list repos in the same order as the sidebar (pinned first, then the current sort mode), so that I can find a repo in it by scanning where I expect.
25. As a developer, I want to type in the manage popover to filter its list, so that curating a large scan doesn't turn into scrolling.
26. As a developer, I want the filter remembered only while the popover is open, so that each curation session starts fresh.

### Polish

27. As a developer, I want the selected repo to stay selected when I hide it, so that hiding never changes what I'm looking at — the details pane keeps showing it until I pick something else.
28. As a developer, I want hidden rows excluded from arrow-key navigation, so that the keyboard doesn't reach repos I can't see.
29. As a VoiceOver user, I want the manage button labelled, its rows announced with the repo name and hidden state, and the toggles labelled "Hide <repo>" / "Show <repo>", so that curation works by keyboard and screen reader.
30. As a developer, I want the popover to follow the existing menu-popover look and interactions, so that the affordance feels native rather than bolted on.

## Implementation Decisions

- **Settings** gains `hidden_repos`, a list of repo paths alongside `pinned_repos`, with the same rules: kept while the repo is absent from the snapshot, ignored on the way in, not part of `Preferences` so a Settings-window Save can't overwrite it. A `toggle_hidden(repo_path)` method returns whether anything changed. Defaults to empty; old settings files load unchanged.
- **Repo order** gains hiding as an explicit input. The order function's hidden parameter filters before partitioning into pinned and others; the sidebar passes the settings' hidden set, and the menu-bar popover passes an empty set, keeping its list complete. Hiding is applied before the sort, so nothing reshuffles. The order the popover sees is unchanged, byte for byte, by passing empty.
- **Count semantics.** The sidebar's REPOSITORIES · N header counts the ordered list repo_order returns. When the hidden set is non-empty, a "· N hidden" chip renders after the count and opens the manage popover. The popover's own counts and layout are unchanged.
- **UI.** The manage button and its popover mirror the existing repo-sort menu mechanics (same popover ids pattern, same click-outside/Esc dismissal). Popover rows show the repo name and an eye toggle; hidden rows are dimmed with a strikethrough name. The popover gains a filter text field that starts focused; it filters by name as typed and is cleared on close.
- **Selection stays identity-based** as it is today. Hiding the selected repo doesn't move the selection; the details pane keeps showing it, arrow navigation just can't reach it.
- **Frees the row slot** — no hover buttons or menu items on repo rows change. Hiding is a header action, deliberately not a per-row affordance.
- The prototype decision (variant B) came from a throwaway HTML prototype of the real sidebar; its structural facts — chip beside the count, panel keyed off the header, hidden rows struck through — are the decision content here.

## Testing Decisions

A good test drives a public seam and asserts what the user sees or what persists. It doesn't poke private helpers or count git calls.

### Seams (all existing)

1. **Settings seam** — plain unit tests: `toggle_hidden` round trip, the entry surviving an absent repo, a pinned repo keeping its pin while hidden, and Settings-window Save leaving `hidden_repos` untouched. Prior art: the tests around `pinned_repos` and recents-vs-Save.
2. **Core order seam** — `repo_order` tests over canned snapshots: hidden repos leave both the pinned and others lists when passed the hidden set, the sort mode still applies to what remains, and counts come out right; an empty hidden set reproduces today's output exactly, which is what the popover's call guarantees. Prior art: the existing repo-order tests (the order test binary).
3. **UI seam (thin)** — the GPUI test app over a canned snapshot: clicking the header button opens the popover; toggling an eye drops the row from the sidebar (count and chip react) and persists to the settings file; another toggle brings it back; the chip opens the popover; Escape and outside click close it; the filter narrows rows. Prior art: the pin/sort menu tests in the main-window suite, the popover absence tests.

## Out of Scope

- Hiding branches, worktrees, or individual folders from the tree.
- Batching hides into an "archive"/"unhide all" action, or multi-select in the popover.
- Any Settings-window UI for hidden repos.
- Excluding hidden repos from the quick switcher or changing its ranking.
- Excluding hidden repos from Fetch all or background fetch — they're still Grove repos and still fetch.
- Deleting or unregistering repos from the scan.
- Per-repo reasons for hiding (no dates, "archived" labels, or annotations).
- A keyboard shortcut for the manage popover.

## Further Notes

- Count semantics change ripples: existing UI tests asserting "REPOSITORIES · N" with every repos present keep passing; any test that hides a repo must expect the visible count. Ticket writing should remember the header count now means "what the list shows".
- Hidden repos still cost a scan read and still participate in Freshness and Fetch all. If background fetch on hidden dead repos ever becomes noise, "excluded from background fetch" is the natural follow-up — deliberately not the default.
- The menu-bar popover is unchanged on purpose: its search already handles a long list. If it ever wants hiding too, the order function already takes the hidden set — it's a one-call-site change.
- Strikethrough on hidden rows is the strongest "this is deliberate state" signal the popover can carry without adding a section header; if it reads poorly with VO, a spoken "hidden" suffix on the row is the fallback.