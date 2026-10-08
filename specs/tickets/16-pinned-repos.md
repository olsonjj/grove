# 16: Pinned repos

**What to build:** User pins a repo from its sidebar row (hover/selected pin button, or Pin / Unpin in the row's context menu). Pinned repos sit in a "PINNED · N" group above the rest, alphabetical, with a filled pin glyph; group hidden when nothing is pinned. Pins persist by repo path, survive the repo disappearing, and lead the popover too. Introduces the core repo-order function (pinned list + others, others still by name) that all surfaces use; views stop iterating snapshot order directly. See [spec](../sort-and-pin.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] Settings `pinned_repos` (repo paths), default empty; `toggle_pin` returns whether anything changed; Settings window Save leaves it untouched
- [x] Core repo-order function returns pinned (by name) and others (by name); pins for repos not in the snapshot are ignored, not dropped
- [x] Sidebar: fixed REPOSITORIES header; PINNED group leads the list, separator before the rest; repo appears once; REPOSITORIES · N still counts every repo
- [x] Pin button on hover/selection, filled on pinned rows; context menu Pin / Unpin
- [x] Selection kept on pin/unpin; arrow keys cross from pinned group into the rest
- [x] Popover lists pinned repos' groups first
- [x] VoiceOver: "Pin <repo>" / "Unpin <repo>"; pinned rows announced as pinned
- [x] Tests: settings round trip and absent-repo pin; repo-order over fixture repos; UI test pin via button and context menu persists
