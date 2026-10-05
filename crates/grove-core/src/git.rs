//! The only module that runs `git`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub binary: PathBuf,
    pub version: GitVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// No usable `git` (not on PATH, or e.g. the /usr/bin/git shim without Xcode CLT).
    NotFound {
        detail: String,
    },
    TooOld {
        found: GitVersion,
    },
    Timeout {
        args: String,
    },
    Failed {
        args: String,
        stderr: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { detail } => write!(f, "git not found: {detail}"),
            Self::TooOld { found } => write!(f, "git {found} is older than {MIN_VERSION}"),
            Self::Timeout { args } => write!(f, "git {args} timed out"),
            Self::Failed { args, stderr } => write!(f, "git {args} failed: {}", stderr.trim()),
        }
    }
}

impl std::error::Error for GitError {}

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeStatus {
    /// Paths `git status` lists: modified, staged, unmerged and untracked.
    pub changed: usize,
    pub upstream: Upstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Change count and ahead/behind vs `@{u}` (any remote), from one read-only `git status`.
pub fn read_worktree_status(path: &Path) -> Result<WorktreeStatus, GitError> {
    let out = run(Some(path), &["status", "--porcelain=v2", "--branch"], READ)?;
    Ok(parse_status(&out))
}

fn run(dir: Option<&Path>, args: &[&str], timeout: Duration) -> Result<String, GitError> {
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
    match process::run(cmd, timeout) {
        Ok(out) if out.status.success() => Ok(out.stdout),
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

fn parse_status(out: &str) -> WorktreeStatus {
    let mut changed = 0;
    let mut upstream = None;
    let mut ab = None;
    for line in out.lines() {
        let Some(header) = line.strip_prefix("# ") else {
            changed += usize::from(!line.is_empty());
            continue;
        };
        if let Some(name) = header.strip_prefix("branch.upstream ") {
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
    WorktreeStatus { changed, upstream }
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
            "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +1 -2\n\
             1 .M N... 100644 100644 100644 a a f.txt\n2 R. N... 100644 100644 100644 a a R100 new\told\n\
             u UU N... 1 2 3 4 a b c m.txt\n? u.txt\n",
        );
        assert_eq!(s.changed, 4);
        assert_eq!(
            s.upstream,
            Upstream::Tracking {
                name: "origin/main".into(),
                ahead: 1,
                behind: 2
            }
        );
        let gone = parse_status("# branch.oid abc\n# branch.head g\n# branch.upstream origin/g\n");
        assert_eq!(
            gone,
            WorktreeStatus {
                changed: 0,
                upstream: Upstream::Gone {
                    name: "origin/g".into()
                }
            }
        );
        let detached = parse_status("# branch.oid abc\n# branch.head (detached)\n? x\n");
        assert_eq!(detached.upstream, Upstream::None);
        assert_eq!(detached.changed, 1);
    }
}
