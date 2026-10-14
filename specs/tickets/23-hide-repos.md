# 23: Hide repos from the sidebar

**What to build:** A manage-visibility button in the sidebar's REPOSITORIES header, next to the sort menu, opens a popover listing every repo Grove found, in the sidebar's order (pinned first, then the current sort mode), each row with a show/hide eye toggle. Hidden rows are dimmed and struck through. Toggling applies at once: the repo leaves the sidebar (including the PINNED group), the header count drops to what's shown, and a "· N hidden" chip appears that reopens the popover — the chip is absent while nothing is hidden. Hiding is remembered by the repo's path, across restarts and while the repo is absent from the scan, without disturbing its pin; unhiding restores the row exactly as it was. The menu-bar popover and the quick switcher keep listing every repo: repo order gains hiding as an explicit parameter, only the desktop sidebar passes the hidden set, and an empty set reproduces today's output exactly. See [spec](../hide-repos.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Settings gains `hidden_repos` (repo paths; kept while a repo is absent; unknown paths ignored); `toggle_hidden` returns whether anything changed; Settings-window Save leaves it untouched; defaults empty, old settings files load unchanged
- [ ] Repo order takes the hidden set and filters before partitioning into pinned and others; an empty set reproduces today's output exactly; hiding applies before the sort, so hiding one repo never reshuffles the rest
- [ ] Sidebar header: the manage button opens the manage popover listing every repo in sidebar order, with an eye toggle per row; hidden rows dimmed and struck through
- [ ] Toggling applies immediately: hidden row leaves the sidebar and PINNED, header count drops, "· N hidden" chip appears; the chip opens the same popover
- [ ] Hiding the selected repo keeps the selection; hidden rows unreachable by arrow keys
- [ ] Pin preserved through hide/unhide; persistence to the settings file survives restart and the repo's temporary absence
- [ ] Menu-bar popover and quick switcher unchanged: the popover's call passes an empty hidden set, and switcher ranking is untouched
- [ ] Manage popover closes with Escape, an outside click, and its button; its focused filter field narrows rows by name while open and resets on close
- [ ] VoiceOver: the manage button labelled, rows announced with repo name and hidden state, toggles labelled "Hide <repo>" / "Show <repo>"
- [ ] Tests: settings round trip and absent-repo retention; order seam (hidden filters both groups, empty set byte-identical, no reshuffle); thin UI tests over the canned snapshot (open popover, toggle leaves/returns row with header count and chip reacting, persistence to the settings file, chip opens popover, filter narrows)