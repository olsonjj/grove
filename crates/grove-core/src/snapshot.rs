use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use crate::discovery::{RepoRef, discover};
use crate::git::{self, GitError, WorktreeStatus};

const MAX_PARALLEL_REPOS: usize = 8;
const MAX_PARALLEL_READS: usize = 8;

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

    fn worktrees(&self) -> impl Iterator<Item = &Worktree> {
        self.repos.iter().flat_map(|r| &r.worktrees)
    }

    /// Worktrees whose status can be read (folder present).
    pub fn status_targets(&self) -> Vec<PathBuf> {
        self.worktrees()
            .filter(|w| w.presence == Presence::Present)
            .map(|w| w.path.clone())
            .collect()
    }

    /// Stores a status read for the worktree at `path`; false if no such worktree.
    pub fn set_status(&mut self, path: &Path, status: Result<WorktreeStatus, GitError>) -> bool {
        let found = self
            .repos
            .iter_mut()
            .flat_map(|r| &mut r.worktrees)
            .find(|w| w.path == path);
        match found {
            Some(w) => {
                w.status = Some(status);
                true
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSnapshot {
    pub repo: RepoRef,
    /// Folder name, or the code-dir-relative path when another repo shares it.
    pub display_name: String,
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
    pub presence: Presence,
    /// `None` until read (and never for a folder that isn't present).
    pub status: Option<Result<WorktreeStatus, GitError>>,
}

/// Whether a worktree's folder is on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Present,
    /// Folder deleted; git keeps the entry until `git worktree prune`.
    Missing,
    /// On a volume that isn't mounted.
    Unavailable,
}

impl Presence {
    pub fn of(path: &Path) -> Self {
        presence_in(path, Path::new("/Volumes"))
    }

    pub fn is_openable(self) -> bool {
        self == Self::Present
    }

    /// Why it can't be opened.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Present => None,
            Self::Missing => Some("folder is missing (run git worktree prune)"),
            Self::Unavailable => Some("its drive isn't mounted"),
        }
    }
}

/// Missing unless the path sits under `<volumes>/<name>` and that volume is gone.
fn presence_in(path: &Path, volumes: &Path) -> Presence {
    if path.exists() {
        return Presence::Present;
    }
    let volume = path
        .strip_prefix(volumes)
        .ok()
        .and_then(|rest| rest.components().next());
    match volume {
        Some(name) if !volumes.join(name).exists() => Presence::Unavailable,
        _ => Presence::Missing,
    }
}

/// Reads each path's status, at most `MAX_PARALLEL_READS` at a time, handing each result to
/// `each` as it lands. Blocking; run off the UI thread.
pub fn read_statuses(
    paths: &[PathBuf],
    each: impl Fn(&Path, Result<WorktreeStatus, GitError>) + Sync,
) {
    let next = std::sync::atomic::AtomicUsize::new(0);
    thread::scope(|s| {
        for _ in 0..MAX_PARALLEL_READS.min(paths.len()) {
            s.spawn(|| {
                while let Some(path) =
                    paths.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    each(path, git::read_worktree_status(path));
                }
            });
        }
    });
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
    set_display_names(&mut repos);
    Snapshot {
        code_dir: code_dir.to_path_buf(),
        code_dir_exists,
        repos,
    }
}

fn set_display_names(repos: &mut [RepoSnapshot]) {
    let short = |r: &RepoSnapshot| {
        r.repo
            .name
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let shorts: Vec<String> = repos.iter().map(short).collect();
    for (repo, s) in repos.iter_mut().zip(&shorts) {
        let clash = shorts.iter().filter(|o| *o == s).count() > 1;
        repo.display_name = if clash {
            repo.repo.name.clone()
        } else {
            s.clone()
        };
    }
    repos.sort_by_cached_key(|r| (r.display_name.to_lowercase(), r.repo.name.clone()));
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
                    presence: Presence::of(&e.path),
                    status: None,
                    path: e.path,
                    branch: e.branch,
                    head: e.head,
                    detached: e.detached,
                    locked: e.locked.is_some(),
                    prunable: e.prunable.is_some(),
                })
                .collect();
            RepoSnapshot {
                display_name: String::new(),
                repo,
                worktrees,
                error: None,
            }
        }
        Err(e) => RepoSnapshot {
            display_name: String::new(),
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

    /// Rescans the same code directory (blocking). Statuses start unread.
    pub fn refresh_all(&mut self) {
        self.snapshot = Arc::new(scan(&self.snapshot.code_dir));
    }

    /// Reads every present worktree's status, in parallel (blocking).
    pub fn refresh_statuses(&mut self) {
        let results = std::sync::Mutex::new(Vec::new());
        read_statuses(&self.snapshot.status_targets(), |path, status| {
            results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((path.to_path_buf(), status));
        });
        let snapshot = Arc::make_mut(&mut self.snapshot);
        for (path, status) in results.into_inner().unwrap_or_else(|e| e.into_inner()) {
            snapshot.set_status(&path, status);
        }
    }

    /// Re-checks one worktree's folder and rereads its status (blocking); false if unknown.
    pub fn refresh_worktree(&mut self, path: &Path) -> bool {
        let snapshot = Arc::make_mut(&mut self.snapshot);
        let Some(wt) = snapshot
            .repos
            .iter_mut()
            .flat_map(|r| &mut r.worktrees)
            .find(|w| w.path == path)
        else {
            return false;
        };
        wt.presence = Presence::of(path);
        wt.status = wt
            .presence
            .is_openable()
            .then(|| git::read_worktree_status(path));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_tells_missing_from_unmounted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let volumes = tmp.path().join("Volumes");
        std::fs::create_dir_all(volumes.join("ext/code/api")).unwrap();
        let at = |p: &str| presence_in(&volumes.join(p), &volumes);
        assert_eq!(at("ext/code/api"), Presence::Present);
        assert_eq!(at("ext/code/gone"), Presence::Missing);
        assert_eq!(at("unplugged/code/api"), Presence::Unavailable);
        assert_eq!(
            presence_in(&tmp.path().join("deleted"), &volumes),
            Presence::Missing
        );
    }
}
