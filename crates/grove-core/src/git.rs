//! The only module that runs `git`. Everything here is read-only except `fetch`, `pull`,
//! worktree removal and `switch`.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::env::resolved_env;
use crate::process::{self, RunError};

/// 2.20 (Dec 2018) covers everything Grove runs: `worktree list --porcelain` (2.7),
/// `status --porcelain=v2` (2.11), `GIT_OPTIONAL_LOCKS` (2.15) and worktree lock/move fixes.
/// Every Xcode CLT and Homebrew git on macOS 14+ is far newer.
pub const MIN_VERSION: GitVersion = GitVersion {
    major: 2,
    minor: 20,
    patch: 0,
};

const QUICK: Duration = Duration::from_secs(5);
const READ: Duration = Duration::from_secs(15);
/// Generous: a first fetch is slow, and pull's inner fetch dominates pull; both run off
/// the UI thread.
const NETWORK: Duration = Duration::from_secs(5 * 60);
/// Switch: a kill mid-checkout loses nothing (see `switch`), so the write is bounded here
/// like pull, not never-killed like removal.
const SWITCH: Duration = Duration::from_secs(60);
/// Worktree removal, unbounded: killing it mid-delete would leave a half-deleted worktree.
const WRITE: Duration = Duration::MAX;

/// Semantic version components parsed from `git --version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GitVersion {
    /// Major version component.
    pub major: u32,
    /// Minor version component.
    pub minor: u32,
    /// Patch version component.
    pub patch: u32,
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The git binary Grove runs, from `check`.
#[derive(Debug, Clone)]
pub struct GitInfo {
    /// Absolute or PATH-resolved git executable.
    pub binary: PathBuf,
    /// Detected git version.
    pub version: GitVersion,
}

/// A git command that couldn't run or failed. Serializable so the snapshot cache keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum GitError {
    /// No usable `git` (not on PATH, or e.g. the /usr/bin/git shim without Xcode CLT).
    #[error("git not found: {detail}")]
    NotFound {
        /// Detection or launch failure details.
        detail: String,
    },
    /// Detected git is older than [`MIN_VERSION`].
    #[error("git {found} is older than {MIN_VERSION}")]
    TooOld {
        /// Detected version.
        found: GitVersion,
    },
    /// Killed at its deadline; `args` as run, space-joined.
    #[error("git {args} timed out")]
    Timeout {
        /// Space-joined command arguments.
        args: String,
    },
    /// Exited with an unexpected code.
    #[error("git {args} failed: {}", .stderr.trim())]
    Failed {
        /// Space-joined command arguments.
        args: String,
        /// Captured standard error.
        stderr: String,
    },
}

impl GitError {
    /// One line for a status bar: git's first `fatal:`/`error:` message, else its first line.
    pub fn brief(&self) -> String {
        let Self::Failed { stderr, .. } = self else {
            return self.to_string();
        };
        let mut lines = stderr.lines().map(str::trim).filter(|l| !l.is_empty());
        let first = lines.clone().next();
        lines
            .find_map(|l| {
                l.strip_prefix("fatal: ")
                    .or_else(|| l.strip_prefix("error: "))
            })
            .or(first)
            .map_or_else(|| self.to_string(), str::to_string)
    }
}

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeEntry {
    /// Absolute worktree path.
    pub path: PathBuf,
    /// Full HEAD object id, or `None` before the first commit.
    pub head: Option<String>,
    /// Short branch name (`refs/heads/` stripped); `None` when detached or bare.
    pub branch: Option<String>,
    /// Whether HEAD is detached.
    pub detached: bool,
    /// Whether this entry represents a bare repository.
    pub bare: bool,
    /// Optional reason from `git worktree lock`.
    pub locked: Option<String>,
    /// Optional reason the entry is prunable.
    pub prunable: Option<String>,
}

/// One worktree's `git status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeStatus {
    /// Paths `git status` lists (modified, staged, unmerged, untracked), in its order.
    pub files: Vec<ChangedFile>,
    /// Upstream tracking state for the current branch.
    pub upstream: Upstream,
    /// HEAD's commit; `None` on an unborn branch.
    #[serde(default)]
    pub head: Option<String>,
    /// HEAD's committer time, Unix seconds; `None` on an unborn branch or if unreadable.
    #[serde(default)]
    pub head_time: Option<i64>,
    /// Newest modified time among `files` still on disk, Unix seconds; `None` if none are.
    #[serde(default)]
    pub changes_time: Option<i64>,
}

impl WorktreeStatus {
    /// Returns the number of changed paths.
    pub fn changed(&self) -> usize {
        self.files.len()
    }
}

/// One path `git status` lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Dominant staged or unstaged change kind.
    pub change: Change,
    /// Repo-relative; the new path for renames and copies.
    pub path: String,
    /// Source path of a rename or copy.
    pub from: Option<String>,
}

/// One file's change, staged or not (staged wins when both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// File contents or metadata changed.
    Modified,
    /// File was added.
    Added,
    /// File was deleted.
    Deleted,
    /// File was renamed.
    Renamed,
    /// File was copied.
    Copied,
    /// File type changed.
    TypeChanged,
    /// File has unresolved merge conflicts.
    Unmerged,
    /// File is not tracked.
    Untracked,
}

impl Change {
    /// Git's short status code.
    pub fn code(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Unmerged => "U",
            Self::Untracked => "??",
        }
    }

    /// From porcelain v2's `XY` (index, worktree; `.` = unchanged).
    fn from_xy(xy: &str) -> Self {
        let mut it = xy.chars();
        let (x, y) = (it.next().unwrap_or('.'), it.next().unwrap_or('.'));
        match if x == '.' { y } else { x } {
            'A' => Self::Added,
            'D' => Self::Deleted,
            'R' => Self::Renamed,
            'C' => Self::Copied,
            'T' => Self::TypeChanged,
            _ => Self::Modified,
        }
    }
}

/// One of `git log`'s commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Abbreviated commit object id.
    pub short_sha: String,
    /// First line of the commit message.
    pub subject: String,
    /// Committer time, Unix seconds.
    pub time: i64,
}

/// A branch's upstream, from `git status --branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Upstream {
    /// No upstream configured, or HEAD is detached.
    None,
    /// Tracking a remote branch that still exists.
    Tracking {
        /// Remote branch name.
        name: String,
        /// Local commits not in the upstream.
        ahead: u32,
        /// Upstream commits not local.
        behind: u32,
    },
    /// Configured, but its remote-tracking ref no longer exists.
    Gone {
        /// Missing remote branch name.
        name: String,
    },
}

/// The git binary: `$GROVE_GIT` if set (dev/testing override), else `git` on the resolved PATH.
pub fn binary() -> Option<&'static Path> {
    static BINARY: OnceLock<Option<PathBuf>> = OnceLock::new();
    BINARY
        .get_or_init(|| match std::env::var_os("GROVE_GIT") {
            Some(p) => Some(PathBuf::from(p)),
            None => resolved_env().which("git"),
        })
        .as_deref()
}

/// Finds git and checks it's new enough for Grove.
///
/// # Errors
///
/// Returns [`GitError`] when git cannot be found, launched, parsed, or is too old.
pub fn check() -> Result<GitInfo, GitError> {
    let binary = binary().ok_or_else(|| GitError::NotFound {
        detail: format!("no git on PATH {}", resolved_env().path),
    })?;
    let out = run(None, &["--version"], QUICK).map_err(|e| match e {
        GitError::NotFound { .. } => e,
        e => GitError::NotFound {
            detail: e.to_string(),
        },
    })?;
    let version = parse_version(&out).ok_or_else(|| GitError::NotFound {
        detail: format!("unrecognized version output {:?}", out.trim()),
    })?;
    if version < MIN_VERSION {
        return Err(GitError::TooOld { found: version });
    }
    Ok(GitInfo {
        binary: binary.to_path_buf(),
        version,
    })
}

/// `git worktree list --porcelain`, main worktree (or the bare repo) first.
///
/// # Errors
///
/// Returns [`GitError`] when git cannot run or exits unsuccessfully.
pub fn worktree_list(repo: &Path) -> Result<Vec<WorktreeEntry>, GitError> {
    let out = run(Some(repo), &["worktree", "list", "--porcelain"], READ)?;
    Ok(parse_worktree_list(&out))
}

/// Absolute, canonical path of the repo's common git dir (shared by all its worktrees).
///
/// # Errors
///
/// Returns [`GitError`] when git cannot resolve the common directory.
pub fn common_dir(path: &Path) -> Result<PathBuf, GitError> {
    let out = run(Some(path), &["rev-parse", "--git-common-dir"], QUICK)?;
    let dir = path.join(out.trim_end_matches(['\n', '\r']));
    Ok(dir.canonicalize().unwrap_or(dir))
}

/// Changed files and ahead/behind vs `@{u}` (any remote) from a read-only `git status`,
/// plus the time of the HEAD commit it reports and of the newest changed file.
///
/// # Errors
///
/// Returns [`GitError`] when status cannot be read.
pub fn read_worktree_status(path: &Path) -> Result<WorktreeStatus, GitError> {
    read_status(path, &[])
}

/// What `git worktree remove` would lose: changed entries, each untracked file (a nested
/// repo stays one `dir/` entry), ignored paths (an ignored folder is one `dir/` entry), and
/// whether any submodule changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RemovalStatus {
    pub modified: usize,
    pub untracked: Vec<String>,
    pub ignored: Vec<String>,
    pub submodule_changed: bool,
}

pub(crate) fn read_removal_status(path: &Path) -> Result<RemovalStatus, GitError> {
    let out = run(
        Some(path),
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
        READ,
    )?;
    Ok(parse_removal_status(&out))
}

fn parse_removal_status(out: &str) -> RemovalStatus {
    let mut status = RemovalStatus::default();
    let mut fields = out.split('\0');
    while let Some(field) = fields.next() {
        let Some((tag, body)) = field.split_once(' ') else {
            continue;
        };
        match tag {
            "?" => status.untracked.push(body.to_string()),
            "!" => status.ignored.push(body.to_string()),
            "1" | "2" | "u" => {
                if tag == "2" {
                    fields.next();
                }
                status.modified += 1;
                // XY sub ...; sub is "S..." for a submodule.
                status.submodule_changed |=
                    body.split(' ').nth(1).is_some_and(|s| s.starts_with('S'));
            }
            _ => {}
        }
    }
    status
}

fn read_status(path: &Path, extra: &[&str]) -> Result<WorktreeStatus, GitError> {
    let mut args = vec!["status", "--porcelain=v2", "--branch", "-z"];
    args.extend_from_slice(extra);
    let out = run(Some(path), &args, READ)?;
    let mut status = parse_status(&out);
    status.head_time = status
        .head
        .as_deref()
        .and_then(|oid| commit_time(path, oid));
    status.changes_time = changes_time(path, &status.files);
    Ok(status)
}

/// Stats only the listed paths (an untracked folder is one entry); deleted ones are skipped,
/// and so are ones dated in the future, which would otherwise outrank real activity for good.
fn changes_time(worktree: &Path, files: &[ChangedFile]) -> Option<i64> {
    let now = std::time::SystemTime::now();
    files
        .iter()
        .filter_map(|f| {
            std::fs::symlink_metadata(worktree.join(&f.path))
                .ok()?
                .modified()
                .ok()
        })
        .filter(|&t| t <= now)
        .max()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}

fn commit_time(path: &Path, oid: &str) -> Option<i64> {
    run(
        Some(path),
        &[
            "log",
            "-1",
            "--no-show-signature",
            "--format=%ct",
            oid,
            "--",
        ],
        QUICK,
    )
    .ok()?
    .trim()
    .parse()
    .ok()
}

/// `fetch --all --prune`: every remote, deleted remote branches pruned. One of Grove's
/// repo-writing commands; auto maintenance and gc, which fetch would otherwise start,
/// are off (config form, as `--no-auto-maintenance` needs git 2.29).
///
/// # Errors
///
/// Returns [`GitError`] when fetch cannot run, times out, or fails.
pub fn fetch(repo: &Path) -> Result<(), GitError> {
    run(
        Some(repo),
        &write_args(&["fetch", "--all", "--prune", "--quiet"]),
        NETWORK,
    )
    .map(drop)
}

/// `pull --ff-only`, from the worktree itself: fast-forward only, so a diverged branch
/// is refused with git's reason and nothing changes — never a merge commit or conflict.
/// One of Grove's repo-writing commands, with fetch's config and budget (the network
/// fetch inside pull dominates). A killed run is recoverable: a fast-forward is either
/// done or leaves files `git status` reports as modified, and no `MERGE_HEAD` behind.
///
/// # Errors
///
/// Returns [`GitError`] when pull cannot run, times out, or fails (no upstream,
/// diverged, local changes it would overwrite, ...).
pub fn pull(path: &Path) -> Result<(), GitError> {
    run(
        Some(path),
        &write_args(&["pull", "--ff-only", "--quiet"]),
        NETWORK,
    )
    .map(drop)
}

/// Local branch short names, alphabetically. Includes branches no worktree has checked
/// out; which worktree holds a branch is in the snapshot, not this list.
///
/// # Errors
///
/// Returns [`GitError`] when the refs cannot be read.
pub fn branches(path: &Path) -> Result<Vec<String>, GitError> {
    let out = run(
        Some(path),
        &["for-each-ref", "refs/heads", "--format=%(refname:short)"],
        QUICK,
    )?;
    Ok(out.lines().map(str::to_string).collect())
}

/// `switch --quiet -- <branch>`, run from the worktree itself: git carries compatible
/// uncommitted changes over and refuses with its one-line reason when it can't — it
/// never discards anything, and a switch creates or deletes no refs. One of Grove's
/// repo-writing commands, with fetch's config and a 60-second budget: git carries
/// compatible changes or refuses, a kill creates no merge state and loses nothing, and a
/// mid-checkout kill leaves only a stale index lock and files git reports as modified —
/// bounded like pull's, not removal's never-kill.
///
/// # Errors
///
/// Returns [`GitError`] when git cannot run, times out, or refuses the switch (another
/// worktree's branch, changes it would overwrite, no such branch, ...).
pub fn switch(path: &Path, branch: &str) -> Result<(), GitError> {
    run(
        Some(path),
        &write_args(&["switch", "--quiet", "--", branch]),
        SWITCH,
    )
    .map(drop)
}

/// `git worktree remove [--force] <path>`, run from `repo`'s main worktree (or the bare
/// repo). The branch stays. A missing folder's entry goes without `--force`. Never timed out:
/// killing it mid-delete would leave a half-deleted worktree.
///
/// # Errors
///
/// Returns [`GitError`] when git cannot remove the worktree.
pub fn remove_worktree(repo: &Path, path: &Path, force: bool) -> Result<(), GitError> {
    let path = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.extend(["--", &path]);
    run(Some(repo), &args, WRITE).map(drop)
}

fn write_args<'a>(cmd: &[&'a str]) -> Vec<&'a str> {
    // Auto maintenance and gc, which fetch or pull would otherwise start, are off
    // (config form, as `--no-auto-maintenance` needs git 2.29).
    let mut args = vec!["-c", "maintenance.auto=false", "-c", "gc.auto=0"];
    args.extend_from_slice(cmd);
    args
}

/// The last five commits on the worktree's HEAD, newest first; empty before the first commit.
///
/// # Errors
///
/// Returns [`GitError`] when the log cannot be read.
pub fn recent_commits(path: &Path) -> Result<Vec<Commit>, GitError> {
    let out = run(
        Some(path),
        &[
            "log",
            "-5",
            "-z",
            "--no-show-signature",
            "--format=%h%x1f%ct%x1f%s",
            "--ignore-missing",
            "HEAD",
            "--",
        ],
        READ,
    )?;
    Ok(parse_log(&out))
}

/// Which of `paths` (relative to `worktree`) git ignores, as `git status` would: tracked
/// files are never ignored, and anything under an ignored folder is.
///
/// # Errors
///
/// Returns [`GitError`] when `git check-ignore` cannot run.
pub fn ignored(worktree: &Path, paths: &[&Path]) -> Result<HashSet<PathBuf>, GitError> {
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_os_str().as_encoded_bytes());
        input.push(0);
    }
    // Exit 1 just means nothing matched.
    let out = run_with(
        Some(worktree),
        &["check-ignore", "--stdin", "-z"],
        input,
        &[0, 1],
        QUICK,
    )?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn run(dir: Option<&Path>, args: &[&str], timeout: Duration) -> Result<String, GitError> {
    run_with(dir, args, Vec::new(), &[0], timeout)
}

fn run_with(
    dir: Option<&Path>,
    args: &[&str],
    input: Vec<u8>,
    ok_codes: &[i32],
    timeout: Duration,
) -> Result<String, GitError> {
    let joined = args.join(" ");
    let binary = binary().ok_or_else(|| GitError::NotFound {
        detail: "no git on PATH".into(),
    })?;
    let mut cmd = resolved_env().command(binary);
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    match process::run_with_input(cmd, input, timeout) {
        Ok(out) if out.status.code().is_some_and(|c| ok_codes.contains(&c)) => Ok(out.stdout),
        Ok(out) => Err(GitError::Failed {
            args: joined,
            stderr: out.stderr,
        }),
        Err(RunError::Timeout) => Err(GitError::Timeout { args: joined }),
        Err(RunError::Spawn(e)) => Err(GitError::NotFound {
            detail: format!("{}: {e}", binary.display()),
        }),
    }
}

fn parse_version(out: &str) -> Option<GitVersion> {
    // "git version 2.50.1 (Apple Git-155)" / "git version 2.39.3.windows.1"
    let rest = out.trim().strip_prefix("git version ")?;
    let mut parts = rest
        .split(|c: char| !c.is_ascii_digit())
        .take(3)
        .map(|p| p.parse::<u32>().ok());
    Some(GitVersion {
        major: parts.next()??,
        minor: parts.next().flatten().unwrap_or(0),
        patch: parts.next().flatten().unwrap_or(0),
    })
}

fn parse_worktree_list(out: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in out.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key == "worktree" {
            entries.extend(current.take());
            current = Some(WorktreeEntry {
                path: PathBuf::from(value),
                ..Default::default()
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        match key {
            "HEAD" => entry.head = Some(value.to_string()),
            "branch" => {
                entry.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).into())
            }
            "detached" => entry.detached = true,
            "bare" => entry.bare = true,
            "locked" => entry.locked = Some(value.to_string()),
            "prunable" => entry.prunable = Some(value.to_string()),
            _ => {}
        }
    }
    entries.extend(current);
    entries
}

/// Parses `status --porcelain=v2 --branch -z`.
fn parse_status(out: &str) -> WorktreeStatus {
    let mut files = Vec::new();
    let mut upstream = None;
    let mut ab = None;
    let mut head = None;
    let mut fields = out.split('\0');
    while let Some(field) = fields.next() {
        let Some(header) = field.strip_prefix("# ") else {
            files.extend(parse_entry(field, &mut fields));
            continue;
        };
        if let Some(oid) = header.strip_prefix("branch.oid ") {
            head = Some(oid.to_string()).filter(|oid| oid != "(initial)");
        } else if let Some(name) = header.strip_prefix("branch.upstream ") {
            upstream = Some(name.to_string());
        } else if let Some(counts) = header.strip_prefix("branch.ab ") {
            // "+1 -2"
            let mut it = counts
                .split(' ')
                .map(|c| c.get(1..).and_then(|n| n.parse::<u32>().ok()));
            ab = it.next().flatten().zip(it.next().flatten());
        }
    }
    // git omits branch.ab when the upstream ref is missing.
    let upstream = match (upstream, ab) {
        (None, _) => Upstream::None,
        (Some(name), Some((ahead, behind))) => Upstream::Tracking {
            name,
            ahead,
            behind,
        },
        (Some(name), None) => Upstream::Gone { name },
    };
    WorktreeStatus {
        files,
        upstream,
        head,
        head_time: None,
        changes_time: None,
    }
}

/// One file entry; a rename or copy takes its source path from the next field.
fn parse_entry<'a>(
    field: &'a str,
    rest: &mut impl Iterator<Item = &'a str>,
) -> Option<ChangedFile> {
    let (tag, body) = field.split_once(' ')?;
    let file = |change, path: &str, from| ChangedFile {
        change,
        path: path.to_string(),
        from,
    };
    // Paths may contain spaces, so split only the fixed fields before them.
    match tag {
        // XY sub mH mI mW hH hI path
        "1" => {
            let mut f = body.splitn(8, ' ');
            let change = Change::from_xy(f.next()?);
            Some(file(change, f.nth(6)?, None))
        }
        // XY sub mH mI mW hH hI Xscore path, then origPath
        "2" => {
            let from = rest.next().map(str::to_string);
            let mut f = body.splitn(9, ' ');
            let change = Change::from_xy(f.next()?);
            Some(file(change, f.nth(7)?, from))
        }
        // XY sub m1 m2 m3 mW h1 h2 h3 path
        "u" => Some(file(Change::Unmerged, body.splitn(10, ' ').nth(9)?, None)),
        "?" => Some(file(Change::Untracked, body, None)),
        _ => None,
    }
}

/// Parses `log -z --format=%h%x1f%ct%x1f%s`.
fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\0')
        .filter_map(|record| {
            let mut f = record.splitn(3, '\x1f');
            let short_sha = f.next().filter(|s| !s.is_empty())?.to_string();
            let time = f.next()?.parse().ok()?;
            Some(Commit {
                short_sha,
                time,
                subject: f.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        let v = |s| parse_version(s).map(|v| v.to_string());
        assert_eq!(v("git version 2.50.1 (Apple Git-155)\n").unwrap(), "2.50.1");
        assert_eq!(v("git version 2.39.3.windows.1").unwrap(), "2.39.3");
        assert_eq!(v("git version 2.4").unwrap(), "2.4.0");
        assert!(parse_version("xcrun: error: invalid active developer path").is_none());
        assert!(parse_version("git version 2.19.9").unwrap() < MIN_VERSION);
    }

    #[test]
    fn parses_worktree_porcelain() {
        let out = "worktree /r\nbare\n\nworktree /r/wt a\nHEAD abc\nbranch refs/heads/feature/x\nlocked\n\nworktree /w2\nHEAD def\ndetached\nprunable gitdir file points to non-existent location\n\n";
        let e = parse_worktree_list(out);
        assert_eq!(e.len(), 3);
        assert!(e[0].bare);
        assert_eq!(e[1].path, PathBuf::from("/r/wt a"));
        assert_eq!(e[1].branch.as_deref(), Some("feature/x"));
        assert_eq!(e[1].locked.as_deref(), Some(""));
        assert!(e[2].detached && e[2].prunable.is_some());
    }

    #[test]
    fn parses_status_porcelain_v2() {
        let s = parse_status(
            "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +1 -2\0\
             1 .M N... 100644 100644 100644 a a f.txt\0\
             2 R. N... 100644 100644 100644 a a R100 new name\0old\0\
             u UU N... 1 2 3 4 a b c m.txt\0? u v.txt\0",
        );
        let codes: Vec<_> = s
            .files
            .iter()
            .map(|f| (f.change.code(), f.path.as_str(), f.from.as_deref()))
            .collect();
        assert_eq!(
            codes,
            [
                ("M", "f.txt", None),
                ("R", "new name", Some("old")),
                ("U", "m.txt", None),
                ("??", "u v.txt", None),
            ]
        );
        assert_eq!(
            s.upstream,
            Upstream::Tracking {
                name: "origin/main".into(),
                ahead: 1,
                behind: 2
            }
        );
        let gone = parse_status("# branch.oid abc\0# branch.head g\0# branch.upstream origin/g\0");
        assert_eq!(
            gone,
            WorktreeStatus {
                files: vec![],
                upstream: Upstream::Gone {
                    name: "origin/g".into()
                },
                head: Some("abc".into()),
                head_time: None,
                changes_time: None,
            }
        );
        let unborn = parse_status("# branch.oid (initial)\0# branch.head main\0");
        assert_eq!(unborn.head, None);
        let detached = parse_status("# branch.oid abc\0# branch.head (detached)\0? x\0");
        assert_eq!(detached.upstream, Upstream::None);
        assert_eq!(detached.changed(), 1);
    }

    #[test]
    fn staged_change_wins_over_worktree_change() {
        let code = |xy| Change::from_xy(xy).code();
        assert_eq!(
            ["A.", "AM", ".M", "M.", "MD", ".D", "D.", ".T"].map(code),
            ["A", "A", "M", "M", "M", "D", "D", "T"]
        );
    }

    #[test]
    fn removal_status_counts_entries_and_spots_changed_submodules() {
        let z = |fields: &[&str]| fields.join("\0") + "\0";
        let out = z(&[
            "1 .M N... 100644 100644 100644 aaa aaa a.txt",
            "2 R. N... 100644 100644 100644 bbb bbb R100 new name.txt",
            "old name.txt",
            "? notes/one.txt",
            "? nested/",
            "! node_modules/",
        ]);
        assert_eq!(
            parse_removal_status(&out),
            RemovalStatus {
                modified: 2,
                untracked: vec!["notes/one.txt".into(), "nested/".into()],
                ignored: vec!["node_modules/".into()],
                submodule_changed: false,
            }
        );
        let sub = z(&["1 .M SC.. 160000 160000 160000 ccc ccc lib"]);
        assert!(parse_removal_status(&sub).submodule_changed);
    }

    #[test]
    fn brief_error_picks_gits_reason() {
        let failed = |stderr: &str| GitError::Failed {
            args: "fetch --all --prune --quiet".into(),
            stderr: stderr.into(),
        };
        let offline = "Fetching origin\nfatal: unable to access 'https://x/': Could not resolve host: x\nerror: could not fetch origin\n";
        assert_eq!(
            failed(offline).brief(),
            "unable to access 'https://x/': Could not resolve host: x"
        );
        assert_eq!(failed("  odd output\n").brief(), "odd output");
        assert_eq!(
            failed("").brief(),
            "git fetch --all --prune --quiet failed: "
        );
        let timeout = GitError::Timeout {
            args: "fetch".into(),
        };
        assert_eq!(timeout.brief(), "git fetch timed out");
    }

    #[test]
    fn parses_log_with_special_characters() {
        let out = "abc1234\x1f1700000000\x1ffix: \"quotes\" $HOME `tick` | pipe\0\
                   def5678\x1f1690000000\x1f\0";
        assert_eq!(
            parse_log(out),
            [
                Commit {
                    short_sha: "abc1234".into(),
                    subject: "fix: \"quotes\" $HOME `tick` | pipe".into(),
                    time: 1_700_000_000,
                },
                Commit {
                    short_sha: "def5678".into(),
                    subject: String::new(),
                    time: 1_690_000_000,
                },
            ]
        );
        assert!(parse_log("").is_empty());
    }
}
