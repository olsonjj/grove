//! The order every surface lists repos in.

use std::cmp::{Ordering, Reverse};
use std::path::PathBuf;

use crate::settings::{LastOpened, SortMode};
use crate::snapshot::{RepoSnapshot, Snapshot, Worktree};

/// Repos split into the pinned group and the rest; each repo is in exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoOrder<'a> {
    /// By name.
    pub pinned: Vec<&'a RepoSnapshot>,
    /// By the sort mode.
    pub others: Vec<&'a RepoSnapshot>,
}

impl<'a> RepoOrder<'a> {
    /// Pinned, then the others.
    pub fn iter(&self) -> impl Iterator<Item = &'a RepoSnapshot> + '_ {
        self.pinned.iter().chain(&self.others).copied()
    }
}

/// `pinned` holds repo paths; pins for repos not in `snapshot` are ignored. `hidden` holds
/// repo paths to leave out of both groups; the menu-bar popover passes an empty set to keep
/// its list complete.
pub fn repo_order<'a>(
    snapshot: &'a Snapshot,
    pinned: &[PathBuf],
    hidden: &[PathBuf],
    last_opened: &LastOpened,
    sort: SortMode,
) -> RepoOrder<'a> {
    // The snapshot is already in name order, and the sort is stable. Filtering for hidden
    // repos happens before the partition and sort, so hiding one repo never reshuffles
    // the rest.
    let (pinned, mut others): (Vec<_>, Vec<_>) = snapshot
        .repos
        .iter()
        .filter(|r| !hidden.contains(&r.repo.path))
        .partition(|r| pinned.contains(&r.repo.path));
    if sort == SortMode::Recent {
        others.sort_by_cached_key(|r| recency(repo_activity(r, last_opened)));
    }
    RepoOrder { pinned, others }
}

/// When the worktree was last worked in: the latest of when Grove opened it, its HEAD
/// commit and its newest changed file.
pub fn activity_time(wt: &Worktree, last_opened: &LastOpened) -> Option<i64> {
    [
        last_opened.get(&wt.path).copied(),
        wt.head_time,
        wt.changes_time,
    ]
    .into_iter()
    .flatten()
    .max()
}

/// The latest activity among the repo's worktrees.
pub fn repo_activity(repo: &RepoSnapshot, last_opened: &LastOpened) -> Option<i64> {
    repo.worktrees
        .iter()
        .filter_map(|wt| activity_time(wt, last_opened))
        .max()
}

/// Newest first, unknown last. `Equal` in Name mode, so callers fall back to name order.
pub(crate) fn by_activity(sort: SortMode, a: Option<i64>, b: Option<i64>) -> Ordering {
    match sort {
        SortMode::Recent => recency(a).cmp(&recency(b)),
        SortMode::Name => Ordering::Equal,
    }
}

/// Sort key: newest first, unknown last.
fn recency(t: Option<i64>) -> (bool, Reverse<Option<i64>>) {
    (t.is_none(), Reverse(t))
}
