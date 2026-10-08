//! The only module that runs `git`. Everything here is read-only except `fetch`.

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
/// Generous: a first fetch of a big repo is slow, and it runs off the UI thread.
const FETCH: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
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
    pub binary: PathBuf,
    pub version: GitVersion,
}

/// A git command that couldn't run or failed. Serializable so the snapshot cache keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum GitError {
    /// No usable `git` (not on PATH, or e.g. the /usr/bin/git shim without Xcode CLT).
    #[error("git not found: {detail}")]
    NotFound { detail: String },
    #[error("git {found} is older than {MIN_VERSION}")]
    TooOld { found: GitVersion },
    /// Killed at its deadline; `args` as run, space-joined.
    #[error("git {args} timed out")]
    Timeout { args: String },
    /// Exited with an unexpected code.
    #[error("git {args} failed: {}", .stderr.trim())]
    Failed { args: String, stderr: String },
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
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name (`refs/heads/` stripped); `None` when detached or bare.
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
}

/// One worktree's `git status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeStatus {
    /// Paths `git status` lists (modified, staged, unmerged, untracked), in its order.
    pub files: Vec<ChangedFile>,
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
    pub fn changed(&self) -> usize {
        self.files.len()
    }
}

/// One path `git status` lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub change: Change,
    /// Repo-relative; the new path for renames and copies.
    pub path: String,
    /// Source path of a rename or copy.
    pub from: Option<String>,
}

/// One file's change, staged or not (staged wins when both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
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
    pub short_sha: String,
    pub subject: String,
    /// Committer time, Unix seconds.
    pub time: i64,
}

/// A branch's upstream, from `git status --branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Upstream {
    /// No upstream configured, or HEAD is detached.
    None,
    Tracking {
        name: String,
        ahead: u32,
        behind: u32,
    },
    /// Configured, but its remote-tracking ref no longer exists.
    Gone { name: String },
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
pub fn worktree_list(repo: &Path) -> Result<Vec<WorktreeEntry>, GitError> {
    let out = run(Some(repo), &["worktree", "list", "--porcelain"], READ)?;
    Ok(parse_worktree_list(&out))
}

/// Absolute, canonical path of the repo's common git dir (shared by all its worktrees).
pub fn common_dir(path: &Path) -> Result<PathBuf, GitError> {
    let out = run(Some(path), &["rev-parse", "--git-common-dir"], QUICK)?;
    let dir = path.join(out.trim_end_matches(['\n', '\r']));
    Ok(dir.canonicalize().unwrap_or(dir))
}

/// Changed files and ahead/behind vs `@{u}` (any remote) from a read-only `git status`,
/// plus the time of the HEAD commit it reports and of the newest changed file.
pub fn read_worktree_status(path: &Path) -> Result<WorktreeStatus, GitError> {
    let out = run(
        Some(path),
        &["status", "--porcelain=v2", "--branch", "-z"],
        READ,
    )?;
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

/// `fetch --all --prune`: every remote, deleted remote branches pruned. Grove's only
/// command that writes to a repo; auto maintenance and gc, which fetch would otherwise
/// start, are off (config form, as `--no-auto-maintenance` needs git 2.29).
pub fn fetch(repo: &Path) -> Result<(), GitError> {
    run(Some(repo), FETCH_ARGS, FETCH).map(drop)
}

const FETCH_ARGS: &[&str] = &[
    "-c",
    "maintenance.auto=false",
    "-c",
    "gc.auto=0",
    "fetch",
    "--all",
    "--prune",
    "--quiet",
];

/// The last three commits on the worktree's HEAD, newest first; empty before the first commit.
pub fn recent_commits(path: &Path) -> Result<Vec<Commit>, GitError> {
    let out = run(
        Some(path),
        &[
            "log",
            "-3",
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
