use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use crate::discovery::{RepoRef, discover};
use crate::git::{self, GitError};

const MAX_PARALLEL_REPOS: usize = 8;

/// Everything Grove knows about the code directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub code_dir: PathBuf,
    pub code_dir_exists: bool,
    pub repos: Vec<RepoSnapshot>,
}

impl Snapshot {
    pub fn repo(&self, name: &str) -> Option<&RepoSnapshot> {
        self.repos.iter().find(|r| r.repo.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSnapshot {
    pub repo: RepoRef,
    /// Main worktree first (normal repos), then linked worktrees in git's order.
    /// Bare repos have only linked worktrees.
    pub worktrees: Vec<Worktree>,
    pub error: Option<GitError>,
}

impl RepoSnapshot {
    pub fn name(&self) -> &str {
        &self.repo.name
    }

    pub fn worktree_count(&self) -> usize {
        self.worktrees.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub is_main: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// Builds a fresh snapshot: discovery plus each repo's worktrees (including ones outside
/// `code_dir`). Blocking; run off the UI thread.
pub fn scan(code_dir: &Path) -> Snapshot {
    let code_dir_exists = code_dir.is_dir();
    let refs = if code_dir_exists {
        discover(code_dir)
    } else {
        Vec::new()
    };
    let mut repos = Vec::with_capacity(refs.len());
    for chunk in refs.chunks(MAX_PARALLEL_REPOS) {
        thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|r| s.spawn(move || read_repo(r.clone())))
                .collect();
            repos.extend(handles.into_iter().filter_map(|h| h.join().ok()));
        });
    }
    Snapshot {
        code_dir: code_dir.to_path_buf(),
        code_dir_exists,
        repos,
    }
}

fn read_repo(repo: RepoRef) -> RepoSnapshot {
    match git::worktree_list(&repo.path) {
        Ok(entries) => {
            let worktrees = entries
                .into_iter()
                .enumerate()
                .filter(|(_, e)| !e.bare)
                .map(|(i, e)| Worktree {
                    is_main: i == 0,
                    path: e.path,
                    branch: e.branch,
                    head: e.head,
                    detached: e.detached,
                    locked: e.locked.is_some(),
                    prunable: e.prunable.is_some(),
                })
                .collect();
            RepoSnapshot {
                repo,
                worktrees,
                error: None,
            }
        }
        Err(e) => RepoSnapshot {
            repo,
            worktrees: Vec::new(),
            error: Some(e),
        },
    }
}

/// Owns the current snapshot for one code directory.
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    snapshot: Arc<Snapshot>,
}

impl SnapshotStore {
    /// Scans `code_dir` (blocking).
    pub fn new(code_dir: &Path) -> Self {
        Self::from_snapshot(scan(code_dir))
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
        }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.clone()
    }

    /// Rescans the same code directory (blocking).
    pub fn refresh_all(&mut self) {
        self.snapshot = Arc::new(scan(&self.snapshot.code_dir));
    }
}
