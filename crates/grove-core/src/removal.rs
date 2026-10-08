//! Whether and how a linked worktree can be removed.

use std::path::{Path, PathBuf};

use crate::git::{self, GitError, RemovalStatus};
use crate::snapshot::{Presence, Worktree};

/// What removing a worktree would do, from a fresh status read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalPlan {
    /// Why not.
    NotAllowed(String),
    /// Folder gone: a plain `git worktree remove` drops just its entry.
    Missing,
    Clean,
    /// Removal needs `--force` and permanently deletes these.
    Dirty {
        modified: usize,
        untracked: usize,
    },
}

const SUBMODULES: &str = "it contains submodules (remove it with git)";

impl RemovalPlan {
    /// What can be decided without touching disk: `None` means read it (see
    /// `read_removal_plan`).
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
            Presence::Missing => Some(Self::Missing),
            Presence::Unavailable => Some(Self::NotAllowed(
                Presence::Unavailable.reason().unwrap_or_default().into(),
            )),
        }
    }

    fn from_status(wt: &Path, fresh: Result<RemovalStatus, GitError>) -> Self {
        let s = match fresh {
            Ok(s) => s,
            Err(e) => return Self::NotAllowed(format!("couldn't read its status: {}", e.brief())),
        };
        if s.submodule_changed {
            return Self::NotAllowed(SUBMODULES.into());
        }
        // `-uall` lists files, so a folder entry is a repo git won't look inside.
        if let Some(dir) = s
            .untracked
            .iter()
            .find(|p| p.ends_with('/') && wt.join(p).join(".git").exists())
        {
            return Self::NotAllowed(format!(
                "{dir} inside it is a separate Git repository (remove it with git)"
            ));
        }
        match (s.modified, s.untracked.len()) {
            (0, 0) => Self::Clean,
            (modified, untracked) => Self::Dirty {
                modified,
                untracked,
            },
        }
    }

    /// Only a dirty worktree is removed with `--force`.
    pub fn force(&self) -> bool {
        matches!(self, Self::Dirty { .. })
    }
}

/// The plan from a fresh look at the worktree and its admin folder in `git_dir` (the repo's
/// common git dir) (blocking).
pub fn read_removal_plan(git_dir: &Path, wt: &Worktree) -> RemovalPlan {
    let plan = RemovalPlan::gate(wt);
    if matches!(plan, Some(RemovalPlan::NotAllowed(_))) {
        return plan.unwrap_or(RemovalPlan::Clean);
    }
    // Initialized submodules keep their repos here; `--force` would delete them, and a
    // missing worktree's entry would take them along.
    if admin_dir(git_dir, &wt.path).is_some_and(|a| a.join("modules").exists()) {
        return RemovalPlan::NotAllowed(SUBMODULES.into());
    }
    match plan {
        Some(plan) => plan,
        None => RemovalPlan::from_status(&wt.path, git::read_removal_status(&wt.path)),
    }
}

/// `<git_dir>/worktrees/<id>`, whose `gitdir` file points at `worktree`'s `.git`.
fn admin_dir(git_dir: &Path, worktree: &Path) -> Option<PathBuf> {
    std::fs::read_dir(git_dir.join("worktrees"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|dir| {
            std::fs::read_to_string(dir.join("gitdir"))
                .is_ok_and(|s| Path::new(s.trim_end()).parent() == Some(worktree))
        })
}
