//! Whether and how a linked worktree can be removed.

use std::path::{Component, Path, PathBuf};

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
/// common git dir) (blocking). `wt`'s presence is rechecked: a folder can come back.
pub fn read_removal_plan(git_dir: &Path, wt: &Worktree) -> RemovalPlan {
    let wt = &Worktree {
        presence: Presence::of(&wt.path),
        ..wt.clone()
    };
    let plan = RemovalPlan::gate(wt);
    if matches!(plan, Some(RemovalPlan::NotAllowed(_))) {
        return plan.unwrap_or(RemovalPlan::Clean);
    }
    // Initialized submodules keep their repos here; `--force` would delete them, and a
    // missing worktree's entry would take them along.
    if admin_dir(git_dir, &wt.path).is_some_and(|a| a.join("modules").exists()) {
        return RemovalPlan::NotAllowed(SUBMODULES.into());
    }
    if let Some(plan) = plan {
        return plan;
    }
    let status = git::read_removal_status(&wt.path);
    // Ignored files go even without --force, so a repo inside an ignored folder would too.
    let nested = status
        .as_ref()
        .ok()
        .and_then(|s| nested_repo(&wt.path, &s.ignored));
    match nested {
        Some(dir) => RemovalPlan::NotAllowed(format!(
            "{dir} inside it is a separate Git repository (remove it with git)"
        )),
        None => RemovalPlan::from_status(&wt.path, status),
    }
}

/// `<git_dir>/worktrees/<id>`: from the worktree's `.git` file while it's there, else the
/// admin folder whose `gitdir` points back at it. Either may be relative
/// (`worktree.useRelativePaths`).
fn admin_dir(git_dir: &Path, worktree: &Path) -> Option<PathBuf> {
    let pointer = std::fs::read_to_string(worktree.join(".git")).ok();
    if let Some(dir) = pointer
        .as_deref()
        .and_then(|s| s.trim_end().strip_prefix("gitdir: "))
    {
        return Some(normalize(&worktree.join(dir)));
    }
    std::fs::read_dir(git_dir.join("worktrees"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|dir| {
            std::fs::read_to_string(dir.join("gitdir")).is_ok_and(|s| {
                normalize(&dir.join(s.trim_end())).parent() == Some(&normalize(worktree))
            })
        })
}

/// Resolves `.` and `..` without touching disk (the path may be gone).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The first folder at or under `ignored`'s folder entries holding a `.git`, as `dir/`
/// relative to `root`. Symlinks aren't followed.
fn nested_repo(root: &Path, ignored: &[String]) -> Option<String> {
    let mut stack: Vec<PathBuf> = ignored
        .iter()
        .filter(|p| p.ends_with('/'))
        .map(|p| root.join(p))
        .collect();
    while let Some(dir) = stack.pop() {
        if std::fs::symlink_metadata(dir.join(".git")).is_ok() {
            let rel = dir.strip_prefix(root).unwrap_or(&dir);
            return Some(format!("{}/", rel.display()));
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        stack.extend(
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path()),
        );
    }
    None
}
