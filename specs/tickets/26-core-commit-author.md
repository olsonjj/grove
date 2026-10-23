# 26: Core: commit record and log read carry the author name

**What to build:** the details pane's recent-commits data gains the commit's author, end to end at the core seam: the commit record grows an author field, the git log read asks for the plain author name (who wrote the change, not the committer who landed it — e.g. `olsonjj`), and the parser carries the new field without failing a read when it is absent or empty. Nothing renders yet; this ticket is verifiable on its own through the core commit reader, so ticket 27 only has to place the name on screen. See [spec](../commit-details-two-line.md).

**Blocked by:** None (can start immediately)

**Status:** done

- [x] The commit record carries an author field alongside short hash, subject, and committer time
- [x] The recent-commits read requests the plain author name via the standard git author-name format spec
- [x] A missing or empty author slot parses to an empty author, never a failed read
- [x] The log parser's unit test covers an author with the existing special-characters cases (populated and empty)
- [x] The temp-repo integration tests assert a commit made by a known identity returns that author name, with the other record fields unchanged
- [x] Existing core tests (five newest first, special characters, linked worktree, empty repo, signed-commit path) pass unchanged