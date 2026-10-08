//! Whether and how a linked worktree can be removed.

use crate::git::{self, Change, WorktreeStatus};
use crate::snapshot::{Presence, StatusResult, Worktree};

/// What removing a worktree would do, from a fresh status read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalPlan {
    /// Why not.
    NotAllowed(String),
    /// Folder gone: `git worktree prune` drops the stale entry.
    Prune,
    Clean,
    /// Removal needs `--force` and permanently deletes these.
    Dirty {
        modified: usize,
        untracked: usize,
    },
}

impl RemovalPlan {
    /// What can be decided without reading status: `None` means read it (see `with_status`).
    pub fn gate(wt: &Worktree) -> Option<Self> {
        if wt.is_main {
            return Some(Self::NotAllowed("it's the main worktree".into()));
        }
        if wt.locked {
            return Some(Self::NotAllowed(
                "it's locked (run git worktree unlock first)".into(),
            ));
        }
        match wt.presence {
            Presence::Present => None,
            Presence::Missing => Some(Self::Prune),
            Presence::Unavailable => Some(Self::NotAllowed(
                Presence::Unavailable.reason().unwrap_or_default().into(),
            )),
        }
    }

    pub fn with_status(wt: &Worktree, fresh: &StatusResult) -> Self {
        if let Some(plan) = Self::gate(wt) {
            return plan;
        }
        match fresh {
            Ok(s) => Self::from_status(s),
            Err(e) => Self::NotAllowed(format!("couldn't read its status: {}", e.brief())),
        }
    }

    fn from_status(s: &WorktreeStatus) -> Self {
        let untracked = s
            .files
            .iter()
            .filter(|f| f.change == Change::Untracked)
            .count();
        let modified = s.files.len() - untracked;
        if s.files.is_empty() {
            Self::Clean
        } else {
            Self::Dirty {
                modified,
                untracked,
            }
        }
    }

    /// Only a dirty worktree is removed with `--force`.
    pub fn force(&self) -> bool {
        matches!(self, Self::Dirty { .. })
    }
}

/// The plan from a fresh status read that lists every untracked file, not just their folders
/// (blocking).
pub fn read_removal_plan(wt: &Worktree) -> RemovalPlan {
    if let Some(plan) = RemovalPlan::gate(wt) {
        return plan;
    }
    RemovalPlan::with_status(wt, &git::read_status_all_untracked(&wt.path))
}
