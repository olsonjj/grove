use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::discovery::{RepoRef, discover};
use crate::git::{self, GitError, WorktreeStatus};
use crate::watch::Changes;

const MAX_PARALLEL_REPOS: usize = 8;
const MAX_PARALLEL_READS: usize = 8;
const MAX_PARALLEL_FETCHES: usize = 8;

/// Everything Grove knows about the code directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub code_dir: PathBuf,
    pub code_dir_exists: bool,
    pub repos: Vec<RepoSnapshot>,
    /// When the last Fetch all finished, Unix seconds.
    #[serde(default)]
    pub last_fetch: Option<i64>,
}

impl Snapshot {
    pub fn repo(&self, name: &str) -> Option<&RepoSnapshot> {
        self.repos.iter().find(|r| r.repo.name == name)
    }

    fn worktrees(&self) -> impl Iterator<Item = &Worktree> {
        self.repos.iter().flat_map(|r| &r.worktrees)
    }

    fn worktree_mut(&mut self, path: &Path) -> Option<&mut Worktree> {
        self.repos
            .iter_mut()
            .flat_map(|r| &mut r.worktrees)
            .find(|w| w.path == path)
    }
}

/// One discovered repo and its worktrees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSnapshot {
    pub repo: RepoRef,
    /// Folder name, or the code-dir-relative path when another repo shares it.
    pub display_name: String,
    /// Main worktree first (normal repos), then linked worktrees in git's order.
    /// Bare repos have only linked worktrees.
    pub worktrees: Vec<Worktree>,
    /// `git worktree list` failed; `worktrees` is empty.
    pub error: Option<GitError>,
    /// The last fetch failed, so ahead/behind and upstream gone are as of an earlier one.
    #[serde(default)]
    pub fetch_error: Option<GitError>,
}

impl RepoSnapshot {
    pub fn name(&self) -> &str {
        &self.repo.name
    }

    pub fn worktree_count(&self) -> usize {
        self.worktrees.len()
    }
}

/// One entry of `git worktree list` (bare entries skipped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub is_main: bool,
    pub locked: bool,
    pub presence: Presence,
    /// `None` until read (and never for a folder that isn't present).
    pub status: Option<StatusResult>,
    /// HEAD's committer time, Unix seconds, from the status reads; `None` until known, on an
    /// unborn branch, and for a folder that isn't present.
    #[serde(default)]
    pub head_time: Option<i64>,
    /// Newest changed file's modified time, Unix seconds, from the status reads; `None` with
    /// nothing changed on disk, until known, and for a folder that isn't present.
    #[serde(default)]
    pub changes_time: Option<i64>,
}

/// HEAD's time after `status` replaces `prev`. A read that couldn't date HEAD keeps the last
/// time while HEAD may not have moved: the read failed, or found the same commit.
fn head_time_after(status: &StatusResult, prev: Option<&Worktree>) -> Option<i64> {
    let prev_time = prev.and_then(|p| p.head_time);
    let prev_head = match prev.and_then(|p| p.status.as_ref()) {
        Some(Ok(s)) => s.head.as_deref(),
        _ => None,
    };
    match status {
        Ok(s) if s.head_time.is_some() => s.head_time,
        Ok(s) if s.head.is_some() && s.head.as_deref() == prev_head => prev_time,
        Ok(_) => None,
        Err(_) => prev_time,
    }
}

/// The changed files' time after `status` replaces `prev`; a failed read keeps the last one.
fn changes_time_after(status: &StatusResult, prev: Option<&Worktree>) -> Option<i64> {
    match status {
        Ok(s) => s.changes_time,
        Err(_) => prev.and_then(|p| p.changes_time),
    }
}

/// Whether a worktree's folder is on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Why it can't be opened. The one wording for missing/unavailable everywhere.
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

/// One worktree's `git status` read.
pub type StatusResult = Result<WorktreeStatus, GitError>;

/// Status reads for present worktrees, stamped when issued (see `SnapshotStore`).
#[derive(Debug, Clone)]
pub struct StatusReads {
    stamp: u64,
    paths: Vec<PathBuf>,
}

impl StatusReads {
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Reads each status, at most `MAX_PARALLEL_READS` at a time, handing each result to
    /// `each` as it lands. Blocking; run off the UI thread.
    pub fn run(&self, each: impl Fn(&Path, StatusResult) + Sync) {
        let next = AtomicUsize::new(0);
        thread::scope(|s| {
            for _ in 0..MAX_PARALLEL_READS.min(self.paths.len()) {
                s.spawn(|| {
                    while let Some(path) = self.paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                        each(path, git::read_worktree_status(path));
                    }
                });
            }
        });
    }

    /// Runs every read, collecting the results (blocking).
    fn collect(&self) -> HashMap<PathBuf, StatusResult> {
        let results = Mutex::new(HashMap::with_capacity(self.paths.len()));
        self.run(|path, status| {
            results
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(path.to_path_buf(), status);
        });
        results.into_inner().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Fills in every present worktree's status (blocking).
fn read_statuses<'a>(worktrees: impl Iterator<Item = &'a mut Worktree>) {
    let mut worktrees: Vec<_> = worktrees.filter(|w| w.presence.is_openable()).collect();
    let reads = StatusReads {
        stamp: 0,
        paths: worktrees.iter().map(|w| w.path.clone()).collect(),
    };
    let mut results = reads.collect();
    for w in &mut worktrees {
        w.status = results.remove(&w.path);
        w.head_time = w.status.as_ref().and_then(|s| head_time_after(s, None));
        w.changes_time = w.status.as_ref().and_then(|s| changes_time_after(s, None));
    }
}

/// A full rescan of the code directory, statuses included.
#[derive(Debug, Clone)]
pub struct ScanRead {
    stamp: u64,
    code_dir: PathBuf,
}

impl ScanRead {
    /// Blocking; run off the UI thread and hand the result to `SnapshotStore::apply_scan`.
    pub fn run(self) -> ScanResult {
        let mut snapshot = scan(&self.code_dir);
        read_statuses(snapshot.repos.iter_mut().flat_map(|r| &mut r.worktrees));
        ScanResult {
            stamp: self.stamp,
            snapshot,
        }
    }
}

/// A finished `ScanRead`, for `SnapshotStore::apply_scan`.
#[derive(Debug, Clone)]
pub struct ScanResult {
    stamp: u64,
    snapshot: Snapshot,
}

/// A reread of one repo's worktree list and every worktree's status.
#[derive(Debug, Clone)]
pub struct RepoRead {
    stamp: u64,
    repo: RepoRef,
}

impl RepoRead {
    /// Blocking; run off the UI thread and hand the result to `SnapshotStore::apply_repo`.
    pub fn run(self) -> RepoResult {
        let mut repo = read_repo(self.repo);
        read_statuses(repo.worktrees.iter_mut());
        RepoResult {
            stamp: self.stamp,
            repo,
        }
    }
}

/// A finished `RepoRead`, for `SnapshotStore::apply_repo`.
#[derive(Debug, Clone)]
pub struct RepoResult {
    stamp: u64,
    repo: RepoSnapshot,
}

/// `fetch --all --prune` for every repo, then a reread of every status; see
/// `SnapshotStore::fetch_read`.
#[derive(Debug, Clone)]
pub struct FetchRead {
    stamp: u64,
    repos: Vec<RepoRef>,
    statuses: StatusReads,
}

impl FetchRead {
    /// Fetches every repo, at most `MAX_PARALLEL_FETCHES` at a time, then rereads every
    /// status. Blocking; run off the UI thread and hand the result to
    /// `SnapshotStore::apply_fetch`.
    pub fn run(self) -> FetchResult {
        let next = AtomicUsize::new(0);
        let results = Mutex::new(Vec::with_capacity(self.repos.len()));
        thread::scope(|s| {
            for _ in 0..MAX_PARALLEL_FETCHES.min(self.repos.len()) {
                s.spawn(|| {
                    while let Some(repo) = self.repos.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let result = git::fetch(&repo.path);
                        results
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push((repo.git_dir.clone(), result));
                    }
                });
            }
        });
        let finished_at = crate::details::unix_now();
        FetchResult {
            stamp: self.stamp,
            finished_at,
            results: results.into_inner().unwrap_or_else(PoisonError::into_inner),
            status_stamp: self.statuses.stamp,
            statuses: self.statuses.collect(),
        }
    }
}

/// A finished `FetchRead`, for `SnapshotStore::apply_fetch`.
#[derive(Debug, Clone)]
pub struct FetchResult {
    stamp: u64,
    finished_at: i64,
    /// By git dir.
    results: Vec<(PathBuf, Result<(), GitError>)>,
    status_stamp: u64,
    statuses: HashMap<PathBuf, StatusResult>,
}

/// How a Fetch all went, for the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchReport {
    pub fetched: usize,
    /// Display name and error, in sidebar order.
    pub failures: Vec<(String, GitError)>,
}

impl FetchReport {
    pub fn message(&self) -> String {
        let repos = |n| if n == 1 { "repo" } else { "repos" };
        match self.failures.as_slice() {
            [] => format!("Fetched {} {}", self.fetched, repos(self.fetched)),
            [(name, e)] => format!("Fetch failed for {name}: {}", e.brief()),
            [(name, e), rest @ ..] => format!(
                "Fetch failed for {name}: {} (and {} more {})",
                e.brief(),
                rest.len(),
                repos(rest.len())
            ),
        }
    }
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
                .map(|r| (r, s.spawn(move || read_repo(r.clone()))))
                .collect();
            // A read that panicked keeps its repo, as unreadable rather than gone.
            repos.extend(handles.into_iter().map(|(r, h)| {
                h.join()
                    .unwrap_or_else(|_| unread(r.clone(), "worktree list read panicked"))
            }));
        });
    }
    set_display_names(&mut repos);
    Snapshot {
        code_dir: code_dir.to_path_buf(),
        code_dir_exists,
        repos,
        last_fetch: None,
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
                    head_time: None,
                    changes_time: None,
                    path: e.path,
                    branch: e.branch,
                    head: e.head,
                    detached: e.detached,
                    locked: e.locked.is_some(),
                })
                .collect();
            RepoSnapshot {
                display_name: String::new(),
                repo,
                worktrees,
                error: None,
                fetch_error: None,
            }
        }
        Err(e) => failed_repo(repo, e),
    }
}

fn failed_repo(repo: RepoRef, e: GitError) -> RepoSnapshot {
    RepoSnapshot {
        display_name: String::new(),
        repo,
        worktrees: Vec::new(),
        error: Some(e),
        fetch_error: None,
    }
}

fn unread(repo: RepoRef, why: &str) -> RepoSnapshot {
    let e = GitError::Failed {
        args: "worktree list --porcelain".into(),
        stderr: why.into(),
    };
    failed_repo(repo, e)
}

/// Owns the current snapshot for one code directory.
///
/// Reads are stamped when issued and run off the store. One lands only if nothing issued
/// later has already landed for the same repo or worktree, so a slow read can't overwrite
/// fresher data; reads issued before a `replace` never land.
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    snapshot: Arc<Snapshot>,
    issued: u64,
    /// Reads stamped at or below this were issued for a replaced snapshot.
    floor: u64,
    /// Stamp of the read each repo's worktree list came from, by git dir.
    repo_stamps: HashMap<PathBuf, u64>,
    /// Stamp of the read each worktree's status came from.
    status_stamps: HashMap<PathBuf, u64>,
}

impl SnapshotStore {
    /// Scans `code_dir` (blocking). Statuses start unread.
    pub fn new(code_dir: &Path) -> Self {
        Self::from_snapshot(scan(code_dir))
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
            issued: 0,
            floor: 0,
            repo_stamps: HashMap::new(),
            status_stamps: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }

    fn stamp(&mut self) -> u64 {
        self.issued += 1;
        self.issued
    }

    /// Whether a read stamped `stamp` is older than the data it would overwrite.
    fn is_stale(&self, stamp: u64, landed: Option<&u64>) -> bool {
        stamp <= self.floor || landed.is_some_and(|&l| l > stamp)
    }

    /// Swaps in a new snapshot (e.g. another code directory); reads still in flight for the
    /// old one are dropped.
    pub fn replace(&mut self, snapshot: Snapshot) {
        self.snapshot = Arc::new(snapshot);
        self.floor = self.issued;
        self.repo_stamps.clear();
        self.status_stamps.clear();
    }

    /// A full rescan of the same code directory; run it off the UI thread and hand the
    /// result to `apply_scan`.
    pub fn scan_read(&mut self) -> ScanRead {
        ScanRead {
            stamp: self.stamp(),
            code_dir: self.snapshot.code_dir.clone(),
        }
    }

    /// Stores a rescan, keeping any repo or status that a later read already refreshed.
    /// False if dropped.
    pub fn apply_scan(&mut self, result: ScanResult) -> bool {
        let ScanResult {
            stamp,
            mut snapshot,
        } = result;
        if stamp <= self.floor || snapshot.code_dir != self.snapshot.code_dir {
            return false;
        }
        for repo in &mut snapshot.repos {
            let git_dir = repo.repo.git_dir.clone();
            let current = self
                .snapshot
                .repos
                .iter()
                .find(|r| r.repo.git_dir == git_dir);
            if let Some(current) = current
                && self.is_stale(stamp, self.repo_stamps.get(&git_dir))
            {
                let display_name = std::mem::take(&mut repo.display_name);
                *repo = RepoSnapshot {
                    display_name,
                    ..current.clone()
                };
                continue;
            }
            repo.fetch_error = current.and_then(|c| c.fetch_error.clone());
            self.merge_statuses(&mut repo.worktrees, stamp);
            self.repo_stamps.insert(git_dir, stamp);
        }
        snapshot.last_fetch = self.snapshot.last_fetch;
        self.snapshot = Arc::new(snapshot);
        true
    }

    /// Rescans the same code directory, statuses included (blocking).
    pub fn refresh_all(&mut self) {
        let result = self.scan_read().run();
        self.apply_scan(result);
    }

    /// A reread of the repo with this git dir, if it's known; run it off the UI thread and
    /// hand the result to `apply_repo`.
    pub fn repo_read(&mut self, git_dir: &Path) -> Option<RepoRead> {
        let repo = self
            .snapshot
            .repos
            .iter()
            .find(|r| r.repo.git_dir == git_dir)?
            .repo
            .clone();
        Some(RepoRead {
            stamp: self.stamp(),
            repo,
        })
    }

    /// Stores a repo reread; true if it changed the snapshot, false if it didn't or was
    /// dropped (stale, or the repo is gone).
    pub fn apply_repo(&mut self, result: RepoResult) -> bool {
        let RepoResult { stamp, mut repo } = result;
        let git_dir = repo.repo.git_dir.clone();
        if self.is_stale(stamp, self.repo_stamps.get(&git_dir)) {
            return false;
        }
        let Some(i) = self
            .snapshot
            .repos
            .iter()
            .position(|r| r.repo.git_dir == git_dir)
        else {
            return false;
        };
        self.merge_statuses(&mut repo.worktrees, stamp);
        let current = &self.snapshot.repos[i];
        repo.display_name = current.display_name.clone();
        repo.fetch_error = current.fetch_error.clone();
        self.repo_stamps.insert(git_dir, stamp);
        if self.snapshot.repos[i] == repo {
            return false;
        }
        Arc::make_mut(&mut self.snapshot).repos[i] = repo;
        true
    }

    /// Rereads one repo's worktrees and statuses (blocking); true if anything changed.
    pub fn refresh_repo(&mut self, git_dir: &Path) -> bool {
        match self.repo_read(git_dir) {
            Some(read) => self.apply_repo(read.run()),
            None => false,
        }
    }

    /// `fetch --all --prune` for every repo, then every status for the new ahead/behind
    /// and upstream gone; run it off the UI thread and hand the result to `apply_fetch`.
    /// The status reads are stamped now, so ones issued during the fetch win; if one of
    /// those predates the fetch's ref updates, the watcher's reread on `refs/` fixes it.
    pub fn fetch_read(&mut self) -> FetchRead {
        FetchRead {
            stamp: self.stamp(),
            repos: self.snapshot.repos.iter().map(|r| r.repo.clone()).collect(),
            statuses: self.status_reads(),
        }
    }

    /// Records each repo's fetch outcome, the reread statuses, and the fetch time if any repo
    /// fetched. A failed repo keeps its last-known counts, marked stale until a fetch
    /// succeeds. `None` if it predates a `replace`.
    pub fn apply_fetch(&mut self, result: FetchResult) -> Option<FetchReport> {
        if result.stamp <= self.floor {
            return None;
        }
        for (path, status) in result.statuses {
            self.apply_status(result.status_stamp, &path, status);
        }
        let mut outcomes: HashMap<PathBuf, Result<(), GitError>> =
            result.results.into_iter().collect();
        let snapshot = Arc::make_mut(&mut self.snapshot);
        let mut report = FetchReport {
            fetched: 0,
            failures: Vec::new(),
        };
        for repo in &mut snapshot.repos {
            match outcomes.remove(&repo.repo.git_dir) {
                Some(Ok(())) => {
                    report.fetched += 1;
                    repo.fetch_error = None;
                }
                Some(Err(e)) => {
                    report.failures.push((repo.display_name.clone(), e.clone()));
                    repo.fetch_error = Some(e);
                }
                // Discovered since the fetch started.
                None => {}
            }
        }
        if report.fetched > 0 {
            snapshot.last_fetch = Some(result.finished_at);
        }
        Some(report)
    }

    /// Fetches every repo, then rereads every status (blocking).
    pub fn fetch_all(&mut self) -> Option<FetchReport> {
        let result = self.fetch_read().run();
        self.apply_fetch(result)
    }

    /// Statuses read at `stamp` lose to ones from later reads already stored.
    fn merge_statuses(&mut self, worktrees: &mut [Worktree], stamp: u64) {
        for w in worktrees {
            let current = self.snapshot.worktrees().find(|c| c.path == w.path);
            if self.is_stale(stamp, self.status_stamps.get(&w.path)) {
                if w.presence.is_openable() {
                    w.status = current.and_then(|c| c.status.clone());
                    w.head_time = current.and_then(|c| c.head_time);
                    w.changes_time = current.and_then(|c| c.changes_time);
                }
            } else if let Some(status) = &w.status {
                w.head_time = head_time_after(status, current);
                w.changes_time = changes_time_after(status, current);
                self.status_stamps.insert(w.path.clone(), stamp);
            }
        }
    }

    /// Status reads for the current snapshot's present worktrees; run them off the UI
    /// thread and hand each result to `apply_status`.
    pub fn status_reads(&mut self) -> StatusReads {
        self.reads_where(|_| true)
    }

    /// Status read for one worktree, if it's known and present.
    pub fn status_read(&mut self, path: &Path) -> StatusReads {
        self.reads_where(|w| w.path == path)
    }

    fn reads_where(&mut self, keep: impl Fn(&Worktree) -> bool) -> StatusReads {
        StatusReads {
            stamp: self.stamp(),
            paths: self
                .snapshot
                .worktrees()
                .filter(|w| w.presence.is_openable() && keep(w))
                .map(|w| w.path.clone())
                .collect(),
        }
    }

    /// Stores a status read; true if it changed the snapshot. False if unchanged, or
    /// dropped: a later read already landed, it predates a `replace`, the worktree is
    /// unknown, or its folder is no longer present.
    pub fn apply_status(&mut self, stamp: u64, path: &Path, status: StatusResult) -> bool {
        if self.is_stale(stamp, self.status_stamps.get(path)) {
            return false;
        }
        let Some(current) = self.snapshot.worktrees().find(|w| w.path == path) else {
            return false;
        };
        if !current.presence.is_openable() {
            return false;
        }
        self.status_stamps.insert(path.to_path_buf(), stamp);
        if current.status.as_ref() == Some(&status) {
            return false;
        }
        let head_time = head_time_after(&status, Some(current));
        let changes_time = changes_time_after(&status, Some(current));
        if let Some(w) = Arc::make_mut(&mut self.snapshot).worktree_mut(path) {
            w.status = Some(status);
            w.head_time = head_time;
            w.changes_time = changes_time;
        }
        true
    }

    /// Records a fresh presence check; a folder that isn't present loses its status. False
    /// if the worktree is unknown.
    pub fn set_presence(&mut self, path: &Path, presence: Presence) -> bool {
        let Some(current) = self.snapshot.worktrees().find(|w| w.path == path) else {
            return false;
        };
        if current.presence != presence
            && let Some(w) = Arc::make_mut(&mut self.snapshot).worktree_mut(path)
        {
            w.presence = presence;
            if !presence.is_openable() {
                w.status = None;
                w.head_time = None;
                w.changes_time = None;
            }
        }
        true
    }

    /// Reads every present worktree's status, in parallel (blocking).
    pub fn refresh_statuses(&mut self) {
        let reads = self.status_reads();
        for (path, status) in reads.collect() {
            self.apply_status(reads.stamp, &path, status);
        }
    }

    /// Re-checks one worktree's folder and rereads its status (blocking); false if unknown.
    pub fn refresh_worktree(&mut self, path: &Path) -> bool {
        let presence = Presence::of(path);
        if !self.set_presence(path, presence) {
            return false;
        }
        if presence.is_openable() {
            let stamp = self.stamp();
            self.apply_status(stamp, path, git::read_worktree_status(path));
        }
        true
    }

    /// Applies a watcher batch (blocking): rereads each changed repo, then each changed
    /// worktree.
    pub fn refresh(&mut self, changes: &Changes) {
        for git_dir in &changes.repos {
            self.refresh_repo(git_dir);
        }
        for path in &changes.worktrees {
            self.refresh_worktree(path);
        }
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
