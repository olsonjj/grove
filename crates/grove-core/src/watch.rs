//! Watches repos and worktrees on disk (FSEvents) and says what to refresh.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::git;
use crate::snapshot::Snapshot;

/// Quiet time that ends a burst of events.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// A burst that never goes quiet is still flushed this often, inside the 2 s target.
const MAX_DELAY: Duration = Duration::from_secs(1);
/// More distinct paths than this in one ignore check: skip checking, just reread status.
const MAX_IGNORE_QUERY: usize = 512;

/// One debounced batch of what to refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Repos, by git dir, whose worktree list, HEADs or refs may have changed.
    pub repos: Vec<PathBuf>,
    /// Worktrees (not in `repos`) whose files or index changed.
    pub worktrees: Vec<PathBuf>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty() && self.worktrees.is_empty()
    }
}

#[derive(Debug)]
pub struct WatchError(notify::Error);

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "can't watch for changes: {}", self.0)
    }
}

impl std::error::Error for WatchError {}

/// Watches each repo's git dir and each present worktree. Events are debounced on a
/// thread of its own, which blocks while nothing happens (no polling). Dropping it stops
/// watching.
pub struct Watcher {
    inner: Mutex<Inner>,
}

struct Inner {
    fs: RecommendedWatcher,
    roots: BTreeSet<PathBuf>,
    tx: Sender<Msg>,
}

enum Msg {
    Paths(Vec<PathBuf>),
    /// FSEvents dropped events under these paths (or anywhere, if none).
    Rescan(Vec<PathBuf>),
    Targets(Targets),
}

impl Watcher {
    /// Starts watching `snapshot`; `on_change` gets each batch on the watcher's thread.
    pub fn start(
        snapshot: &Snapshot,
        on_change: impl FnMut(Changes) + Send + 'static,
    ) -> Result<Self, WatchError> {
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let fs = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                if event.need_rescan() {
                    let _ = events.send(Msg::Rescan(event.paths));
                } else if !event.kind.is_access() {
                    let _ = events.send(Msg::Paths(event.paths));
                }
            }
        })
        .map_err(WatchError)?;
        thread::Builder::new()
            .name("grove-watcher".into())
            .spawn(move || debounce(&rx, on_change))
            .map_err(|e| WatchError(notify::Error::io(e)))?;
        let watcher = Self {
            inner: Mutex::new(Inner {
                fs,
                roots: BTreeSet::new(),
                tx,
            }),
        };
        watcher.update(snapshot);
        Ok(watcher)
    }

    /// Follows worktrees and repos added or removed since the last call. Cheap when the
    /// set is unchanged; otherwise the FSEvents stream restarts, followed by one batch
    /// rereading every watched repo. Blocking.
    pub fn update(&self, snapshot: &Snapshot) {
        let targets = Targets::of(snapshot);
        let roots = targets.roots();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        let mut catch_up = None;
        if roots != inner.roots {
            let mut paths = inner.fs.paths_mut();
            for gone in inner.roots.difference(&roots) {
                let _ = paths.remove(gone);
            }
            let mut watched: BTreeSet<PathBuf> =
                inner.roots.intersection(&roots).cloned().collect();
            for new in roots.difference(&inner.roots) {
                // Fails only if the folder vanished since the snapshot; the next update
                // retries.
                if paths.add(new, RecursiveMode::Recursive).is_ok() {
                    watched.insert(new.clone());
                }
            }
            let _ = paths.commit();
            // Events during a restart are lost: reread everything still watched. Only
            // when the set really changed (a failed add retries on every update), and
            // the reread leaves it as is, so this can't loop.
            if !inner.roots.is_empty() && !watched.is_empty() && watched != inner.roots {
                catch_up = Some(watched.iter().cloned().collect());
            }
            inner.roots = watched;
        }
        let _ = inner.tx.send(Msg::Targets(targets));
        if let Some(roots) = catch_up {
            let _ = inner.tx.send(Msg::Rescan(roots));
        }
    }
}

/// Collects events until `DEBOUNCE` passes quietly (or `MAX_DELAY` since the first), then
/// hands what they mean to `on_change`. Exits when the watcher is dropped.
fn debounce(rx: &Receiver<Msg>, mut on_change: impl FnMut(Changes)) {
    let mut state = State::default();
    let mut pending = Pending::default();
    while let Ok(msg) = rx.recv() {
        state.take(msg, &mut pending);
        if pending.is_empty() {
            continue;
        }
        let first = Instant::now();
        loop {
            let left = MAX_DELAY.saturating_sub(first.elapsed());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(DEBOUNCE.min(left)) {
                Ok(msg) => state.take(msg, &mut pending),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let changes = state.changes(std::mem::take(&mut pending));
        if !changes.is_empty() {
            on_change(changes);
        }
    }
}

/// What the watcher maps event paths onto, from one snapshot. Paths are canonical (FSEvents
/// reports `/private/var/…`, not `/var/…`); ids are the snapshot's own.
#[derive(Debug, Default)]
struct Targets {
    repos: Vec<RepoTarget>,
    worktrees: Vec<WorktreeTarget>,
}

#[derive(Debug)]
struct RepoTarget {
    /// Snapshot id (`RepoRef::git_dir`).
    id: PathBuf,
    canon: PathBuf,
    /// Index into `worktrees`.
    main: Option<usize>,
}

#[derive(Debug)]
struct WorktreeTarget {
    /// Snapshot path.
    path: PathBuf,
    canon: PathBuf,
    /// Index into `repos`.
    repo: usize,
}

impl Targets {
    fn of(snapshot: &Snapshot) -> Self {
        let mut targets = Self::default();
        for repo in &snapshot.repos {
            let ri = targets.repos.len();
            let mut main = None;
            for w in repo.worktrees.iter().filter(|w| w.presence.is_openable()) {
                let Ok(canon) = w.path.canonicalize() else {
                    continue;
                };
                if w.is_main {
                    main = Some(targets.worktrees.len());
                }
                targets.worktrees.push(WorktreeTarget {
                    path: w.path.clone(),
                    canon,
                    repo: ri,
                });
            }
            let id = repo.repo.git_dir.clone();
            targets.repos.push(RepoTarget {
                canon: id.canonicalize().unwrap_or_else(|_| id.clone()),
                id,
                main,
            });
        }
        targets
    }

    /// Fewest folders covering every git dir and worktree (a main worktree holds its git
    /// dir; linked worktrees may sit inside the main one).
    fn roots(&self) -> BTreeSet<PathBuf> {
        let all: BTreeSet<&PathBuf> = self
            .repos
            .iter()
            .map(|r| &r.canon)
            .chain(self.worktrees.iter().map(|w| &w.canon))
            .filter(|p| p.exists())
            .collect();
        all.iter()
            .filter(|p| !all.iter().any(|o| o != *p && p.starts_with(o)))
            .map(|p| (*p).clone())
            .collect()
    }

    fn locate(&self, path: &Path) -> Option<Hit> {
        let worktree = self
            .worktrees
            .iter()
            .enumerate()
            .filter(|(_, w)| path.starts_with(&w.canon))
            .max_by_key(|(_, w)| w.canon.as_os_str().len());
        // A main worktree's `.git` sits inside its folder, but a worktree may also sit
        // inside a bare repo's git dir: the deeper match wins.
        let repo = self
            .repos
            .iter()
            .enumerate()
            .find(|(_, r)| path.starts_with(&r.canon));
        let depth = |p: &Path| p.as_os_str().len();
        if let Some((ri, r)) = repo
            && worktree.is_none_or(|(_, w)| depth(&w.canon) <= depth(&r.canon))
        {
            return self.in_git_dir(ri, path.strip_prefix(&r.canon).ok()?);
        }
        let (wi, w) = worktree?;
        let rel = path.strip_prefix(&w.canon).ok()?;
        if rel.as_os_str().is_empty() {
            return Some(Hit::Status(wi));
        }
        // A linked worktree's `.git` file, or a nested repo's internals: not this worktree's
        // status.
        if rel.components().any(|c| c.as_os_str() == ".git") {
            return None;
        }
        Some(Hit::File(wi, rel.to_path_buf()))
    }

    /// Only files that change what Grove shows count; everything else git writes (objects,
    /// logs, locks, FETCH_HEAD, ORIG_HEAD, fsmonitor state…) is ignored.
    fn in_git_dir(&self, ri: usize, rel: &Path) -> Option<Hit> {
        if rel
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".lock"))
        {
            return None;
        }
        let parts: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
        let parts: Vec<&str> = parts.iter().map(AsRef::as_ref).collect();
        match parts.as_slice() {
            ["index"] => Some(self.repos[ri].main.map_or(Hit::Repo(ri), Hit::Status)),
            ["worktrees", name, "index"] => Some(self.linked(ri, name)),
            ["HEAD" | "packed-refs" | "config"]
            | ["refs", ..]
            | ["info", "exclude"]
            | ["worktrees"]
            | ["worktrees", _]
            | ["worktrees", _, "HEAD" | "locked" | "gitdir"]
            | ["worktrees", _, "refs", ..] => Some(Hit::Repo(ri)),
            _ => None,
        }
    }

    /// The linked worktree whose admin dir is `worktrees/<name>`, via its `gitdir` file.
    fn linked(&self, ri: usize, name: &str) -> Hit {
        let gitdir = self.repos[ri]
            .canon
            .join("worktrees")
            .join(name)
            .join("gitdir");
        fs::read_to_string(gitdir)
            .ok()
            .and_then(|s| Path::new(s.trim_end()).parent()?.canonicalize().ok())
            .and_then(|canon| self.worktrees.iter().position(|w| w.canon == canon))
            .map_or(Hit::Repo(ri), Hit::Status)
    }
}

enum Hit {
    Repo(usize),
    Status(usize),
    /// A file in a worktree, relative to it.
    File(usize, PathBuf),
}

/// Events collected over one debounce.
#[derive(Default)]
struct Pending {
    paths: HashSet<PathBuf>,
    rescan: HashSet<PathBuf>,
    rescan_all: bool,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.rescan.is_empty() && !self.rescan_all
    }
}

#[derive(Default)]
struct State {
    targets: Targets,
}

impl State {
    fn take(&mut self, msg: Msg, pending: &mut Pending) {
        match msg {
            Msg::Paths(paths) => pending.paths.extend(paths),
            Msg::Rescan(paths) if paths.is_empty() => pending.rescan_all = true,
            Msg::Rescan(paths) => pending.rescan.extend(paths),
            Msg::Targets(targets) => self.targets = targets,
        }
    }

    fn changes(&self, pending: Pending) -> Changes {
        let mut repos = self.rescanned(&pending);
        let mut statuses = BTreeSet::new();
        let mut files: HashMap<usize, BTreeSet<PathBuf>> = HashMap::new();
        for path in pending.paths {
            match self.targets.locate(&path) {
                Some(Hit::Repo(r)) => {
                    repos.insert(r);
                }
                Some(Hit::Status(w)) => {
                    statuses.insert(w);
                }
                Some(Hit::File(w, rel)) => {
                    files.entry(w).or_default().insert(rel);
                }
                None => {}
            }
        }
        let worktrees = &self.targets.worktrees;
        // A worktree folder that's gone needs its repo reread for "missing".
        for &w in statuses.iter().chain(files.keys()) {
            if !worktrees[w].canon.exists() {
                repos.insert(worktrees[w].repo);
            }
        }
        for (w, rels) in files {
            if !repos.contains(&worktrees[w].repo) && self.needs_status(w, rels) {
                statuses.insert(w);
            }
        }
        Changes {
            repos: repos
                .iter()
                .map(|&r| self.targets.repos[r].id.clone())
                .collect(),
            worktrees: statuses
                .into_iter()
                .filter(|&w| !repos.contains(&worktrees[w].repo))
                .map(|w| worktrees[w].path.clone())
                .collect(),
        }
    }

    /// Repos with dropped events under (or above) a rescan path; every repo if a rescan
    /// can't be pinned on one.
    fn rescanned(&self, pending: &Pending) -> BTreeSet<usize> {
        let all = || (0..self.targets.repos.len()).collect();
        if pending.rescan_all {
            return all();
        }
        let mut repos = BTreeSet::new();
        for path in &pending.rescan {
            let overlaps = |p: &Path| p.starts_with(path) || path.starts_with(p);
            let hit: Vec<usize> = self
                .targets
                .repos
                .iter()
                .enumerate()
                .filter(|(_, r)| overlaps(&r.canon))
                .map(|(i, _)| i)
                .chain(
                    self.targets
                        .worktrees
                        .iter()
                        .filter(|w| overlaps(&w.canon))
                        .map(|w| w.repo),
                )
                .collect();
            if hit.is_empty() {
                return all();
            }
            repos.extend(hit);
        }
        repos
    }

    /// Whether any of `rels` (changed paths in worktree `w`) is one `git status` would see.
    ///
    /// Asks `git check-ignore` level by level: first the distinct top folders of every
    /// path, then the next level under the ones that aren't ignored, and so on. A flood
    /// inside `node_modules` costs one call for one path. Stops at the first changed file
    /// git doesn't ignore.
    fn needs_status(&self, w: usize, rels: BTreeSet<PathBuf>) -> bool {
        let target = &self.targets.worktrees[w];
        let mut remaining: Vec<PathBuf> = rels.into_iter().collect();
        let mut depth = 0;
        loop {
            if remaining.is_empty() {
                return false;
            }
            depth += 1;
            let level: BTreeSet<PathBuf> = remaining.iter().map(|r| prefix(r, depth)).collect();
            if level.len() > MAX_IGNORE_QUERY {
                return true;
            }
            let query: Vec<&Path> = level.iter().map(PathBuf::as_path).collect();
            let Ok(found) = git::ignored(&target.path, &query) else {
                return true;
            };
            // A changed file (not just a folder on its way) that git sees.
            if remaining
                .iter()
                .any(|r| r.components().count() <= depth && !found.contains(r))
            {
                return true;
            }
            remaining
                .retain(|r| r.components().count() > depth && !found.contains(&prefix(r, depth)));
        }
    }
}

/// `r`'s first `depth` components.
fn prefix(r: &Path, depth: usize) -> PathBuf {
    r.components().take(depth).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/r` with its git dir at `/r/.git`, plus a linked worktree nested at `/r/wt`.
    fn targets() -> Targets {
        Targets {
            repos: vec![RepoTarget {
                id: "/r/.git".into(),
                canon: "/r/.git".into(),
                main: Some(0),
            }],
            worktrees: vec![
                WorktreeTarget {
                    path: "/r".into(),
                    canon: "/r".into(),
                    repo: 0,
                },
                WorktreeTarget {
                    path: "/r/wt".into(),
                    canon: "/r/wt".into(),
                    repo: 0,
                },
            ],
        }
    }

    fn hit(path: &str) -> &'static str {
        match targets().locate(Path::new(path)) {
            Some(Hit::Repo(_)) => "repo",
            Some(Hit::Status(0)) => "main status",
            Some(Hit::Status(_)) => "other status",
            Some(Hit::File(0, _)) => "main file",
            Some(Hit::File(_, _)) => "linked file",
            None => "-",
        }
    }

    #[test]
    fn git_dir_changes_that_matter_refresh_the_repo() {
        for p in [
            "/r/.git/HEAD",
            "/r/.git/packed-refs",
            "/r/.git/config",
            "/r/.git/refs/heads/feature/x",
            "/r/.git/refs/remotes/origin/main",
            "/r/.git/worktrees",
            "/r/.git/worktrees/wt",
            "/r/.git/worktrees/wt/HEAD",
            "/r/.git/worktrees/wt/locked",
        ] {
            assert_eq!(hit(p), "repo", "{p}");
        }
        assert_eq!(hit("/r/.git/index"), "main status");
    }

    #[test]
    fn git_bookkeeping_is_ignored() {
        for p in [
            "/r/.git",
            "/r/.git/index.lock",
            "/r/.git/HEAD.lock",
            "/r/.git/refs/heads/x.lock",
            "/r/.git/FETCH_HEAD",
            "/r/.git/ORIG_HEAD",
            "/r/.git/COMMIT_EDITMSG",
            "/r/.git/objects/ab/cdef",
            "/r/.git/logs/HEAD",
            "/r/.git/worktrees/wt/logs/HEAD",
            "/r/.git/fsmonitor--daemon/cookies/1",
            "/r/wt/.git",
            "/r/vendor/lib/.git/HEAD",
            "/elsewhere/file",
        ] {
            assert_eq!(hit(p), "-", "{p}");
        }
    }

    #[test]
    fn worktree_files_go_to_the_innermost_worktree() {
        assert_eq!(hit("/r/src/main.rs"), "main file");
        assert_eq!(hit("/r/wt/src/main.rs"), "linked file");
        assert_eq!(hit("/r/wt"), "other status");
    }

    #[test]
    fn worktree_inside_bare_repo_gets_its_files() {
        let targets = Targets {
            repos: vec![RepoTarget {
                id: "/b.git".into(),
                canon: "/b.git".into(),
                main: None,
            }],
            worktrees: vec![WorktreeTarget {
                path: "/b.git/feat".into(),
                canon: "/b.git/feat".into(),
                repo: 0,
            }],
        };
        assert!(matches!(
            targets.locate(Path::new("/b.git/feat/new.txt")),
            Some(Hit::File(0, _))
        ));
        assert!(matches!(
            targets.locate(Path::new("/b.git/HEAD")),
            Some(Hit::Repo(0))
        ));
    }

    #[test]
    fn rescans_reread_the_repos_they_touch_or_all() {
        let mut targets = targets();
        targets.repos.push(RepoTarget {
            id: "/s/.git".into(),
            canon: "/s/.git".into(),
            main: None,
        });
        let state = State { targets };
        let rescan = |paths: &[&str]| {
            let pending = Pending {
                rescan: paths.iter().map(PathBuf::from).collect(),
                rescan_all: paths.is_empty(),
                ..Pending::default()
            };
            state.changes(pending).repos
        };
        let r = PathBuf::from("/r/.git");
        let s = PathBuf::from("/s/.git");
        assert_eq!(rescan(&["/r/wt/src"]), std::slice::from_ref(&r));
        assert_eq!(rescan(&["/s"]), std::slice::from_ref(&s));
        assert_eq!(rescan(&["/elsewhere"]), [r.clone(), s.clone()]);
        assert_eq!(rescan(&[]), [r, s]);
    }
}
