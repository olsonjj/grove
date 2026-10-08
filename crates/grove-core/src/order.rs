//! The order every surface lists repos in.

use std::cmp::Reverse;
use std::path::PathBuf;

use crate::settings::SortMode;
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

/// `pinned` holds repo paths; pins for repos not in `snapshot` are ignored.
pub fn repo_order<'a>(snapshot: &'a Snapshot, pinned: &[PathBuf], sort: SortMode) -> RepoOrder<'a> {
    // The snapshot is already in name order, and the sort is stable.
    let (pinned, mut others): (Vec<_>, Vec<_>) = snapshot
        .repos
        .iter()
        .partition(|r| pinned.contains(&r.repo.path));
    if sort == SortMode::Recent {
        // Newest first, unknown last.
        others.sort_by_cached_key(|r| {
            let t = repo_activity(r);
            (t.is_none(), Reverse(t))
        });
    }
    RepoOrder { pinned, others }
}

/// When the worktree was last worked in: the latest of its known activity signals.
pub fn activity_time(wt: &Worktree) -> Option<i64> {
    [wt.head_time].into_iter().flatten().max()
}

/// The latest activity among the repo's worktrees.
pub fn repo_activity(repo: &RepoSnapshot) -> Option<i64> {
    repo.worktrees.iter().filter_map(activity_time).max()
}
