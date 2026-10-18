## Summary

- STATUS gains a `Size` row: the selected worktree's working tree size on disk, in words ("3 kB", "452 MB", "2.5 MB"), at most one decimal. [PR: 34a added in specs/grove-v1.md]

```diff
 STATUS
   Upstream    origin/main
   Sync        2 ahead, 1 behind
   Working tree  2 changed files
+  Size        452 MB
```

The read follows the recent-commits pattern — per selection, off the UI thread, sequence-guarded — keyed on worktree path plus the status read's newest changes mtime, so edits refresh the size while plain re-reads of an unchanged tree reuse the shown value:

```text
DetailsView.sync
  read_commits          # unchanged
  read_sizes            # new: background directory walk
    directory_size      # skips .git, doesn't follow symlinks
  size_line             # "reading…" until it lands, then size_words
```

`.git` is skipped: in the main worktree it's repo data (packs, objects), not the working tree, and in a linked worktree it's a one-line file — excluding it keeps main and linked worktrees measured alike.

## Evidence

- **Before:** `just check` on `branch-switcher` — no Size row, 195 app tests.
  **After:** same gate green (196 app tests + 2 new core unit tests); real-disk UI test asserts the rendered row:

```text
main_window::tests::details_show_the_worktree_size
  fields == [("Upstream", "none"), ("Sync", "no upstream"),
             ("Working tree", "clean"), ("Size", "3 kB")]

details::tests::size_words_rounds_to_the_nearest_named_unit
  512 B → "512 B"; 1.5 kB → "1.5 kB"; 2.5 MB → "2.5 MB"; 452 MB → "452 MB"

details::tests::directory_size_sums_regular_files_without_following_links
  100 + 200 + 30 + symlink(→30) = 330; after deleting target: 300; `.git` dir: skipped
```

## Merge Danger

**Door:** two-way

One detail-pane row plus pure functions in core; a revert restores the previous STATUS exactly.

**Blast Radius:** small

The only user-visible change is the new row on openable worktrees with a read status. Staleness matches recent commits' per-selection philosophy: an untracked-file deletion that doesn't move the changes mtime won't re-read until the next selection or status change.