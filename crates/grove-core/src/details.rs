//! What the details pane says about the selected row.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::git::{Upstream, WorktreeStatus};
use crate::launch::shell_quote;
use crate::snapshot::{RepoSnapshot, Worktree};
use crate::tree::RowId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    /// Full branch name, worktree folder name, or folder prefix.
    pub title: String,
    /// E.g. "Branch · has worktree", "Worktree · detached HEAD".
    pub kind: String,
    /// The worktree the pane's actions apply to: itself, or a branch's first (main first).
    pub worktree: Option<Worktree>,
}

impl RepoSnapshot {
    pub fn details(&self, id: &RowId) -> Option<Details> {
        match id {
            RowId::Branch(name) => {
                let mut wts: Vec<&Worktree> = self
                    .worktrees
                    .iter()
                    .filter(|w| w.branch.as_deref() == Some(name))
                    .collect();
                wts.sort_by(|a, b| b.is_main.cmp(&a.is_main).then(a.path.cmp(&b.path)));
                let kind = match wts.len() {
                    1 => "Branch · has worktree".to_string(),
                    n => format!("Branch · {n} worktrees"),
                };
                Some(Details {
                    title: name.clone(),
                    kind,
                    worktree: wts.first().map(|w| (*w).clone()),
                })
            }
            RowId::Worktree(path) => {
                let wt = self.worktrees.iter().find(|w| &w.path == path)?;
                let kind = if wt.branch.is_none() {
                    "Worktree · detached HEAD"
                } else if wt.is_main {
                    "Worktree · main worktree"
                } else {
                    "Worktree · linked worktree"
                };
                Some(Details {
                    title: wt.path.file_name().map_or_else(
                        || wt.path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    ),
                    kind: kind.into(),
                    worktree: Some(wt.clone()),
                })
            }
            RowId::Folder(prefix) => {
                let n = self
                    .worktrees
                    .iter()
                    .filter_map(|w| w.branch.as_deref())
                    .filter(|b| b.starts_with(prefix.as_str()))
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                Some(Details {
                    title: prefix.clone(),
                    kind: format!("Folder · {n} {}", plural(n, "branch", "branches")),
                    worktree: None,
                })
            }
            RowId::DetachedSection => None,
        }
    }
}

impl Upstream {
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Tracking { name, .. } | Self::Gone { name } => Some(name),
        }
    }

    /// Sync in words: "2 ahead, 1 behind", "up to date", "upstream gone", "no upstream".
    pub fn sync_words(&self) -> String {
        match self {
            Self::None => "no upstream".into(),
            Self::Gone { .. } => "upstream gone".into(),
            Self::Tracking { ahead, behind, .. } => match (ahead, behind) {
                (0, 0) => "up to date".into(),
                (a, 0) => format!("{a} ahead"),
                (0, b) => format!("{b} behind"),
                (a, b) => format!("{a} ahead, {b} behind"),
            },
        }
    }
}

impl WorktreeStatus {
    /// Working tree status in words: "clean", "3 changed files".
    pub fn summary(&self) -> String {
        match self.changed() {
            0 => "clean".into(),
            n => format!("{n} changed {}", plural(n, "file", "files")),
        }
    }
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// `cd` into `path`, shell-quoted for pasting into any POSIX shell.
pub fn cd_command(path: &Path) -> String {
    format!("cd {}", shell_quote(path))
}

/// "just now", "5 minutes ago", "3 days ago", …; a future `then` reads as just now.
pub fn relative_time(then: i64, now: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let secs = now.saturating_sub(then);
    let (n, unit) = if secs < MINUTE {
        return "just now".into();
    } else if secs < HOUR {
        (secs / MINUTE, "minute")
    } else if secs < DAY {
        (secs / HOUR, "hour")
    } else if secs < 7 * DAY {
        (secs / DAY, "day")
    } else if secs < 30 * DAY {
        (secs / (7 * DAY), "week")
    } else if secs < 365 * DAY {
        (secs / (30 * DAY), "month")
    } else {
        (secs / (365 * DAY), "year")
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Seconds since the Unix epoch, for `relative_time`.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn ago(secs: i64) -> String {
        relative_time(NOW - secs, NOW)
    }

    #[test]
    fn relative_time_picks_the_largest_whole_unit() {
        let (m, h, d) = (60, 3600, 86_400);
        let cases = [
            (0, "just now"),
            (59, "just now"),
            (m, "1 minute ago"),
            (59 * m + 59, "59 minutes ago"),
            (h, "1 hour ago"),
            (23 * h, "23 hours ago"),
            (d, "1 day ago"),
            (6 * d, "6 days ago"),
            (7 * d, "1 week ago"),
            (29 * d, "4 weeks ago"),
            (30 * d, "1 month ago"),
            (364 * d, "12 months ago"),
            (365 * d, "1 year ago"),
            (3 * 365 * d, "3 years ago"),
        ];
        for (secs, want) in cases {
            assert_eq!(ago(secs), want, "{secs}s");
        }
    }

    #[test]
    fn future_time_reads_as_just_now() {
        assert_eq!(ago(-3600), "just now");
        assert_eq!(relative_time(i64::MAX, i64::MIN), "just now");
    }

    #[test]
    fn sync_words() {
        let t = |ahead, behind| Upstream::Tracking {
            name: "origin/main".into(),
            ahead,
            behind,
        };
        assert_eq!(t(2, 1).sync_words(), "2 ahead, 1 behind");
        assert_eq!(t(2, 0).sync_words(), "2 ahead");
        assert_eq!(t(0, 1).sync_words(), "1 behind");
        assert_eq!(t(0, 0).sync_words(), "up to date");
        assert_eq!(Upstream::None.sync_words(), "no upstream");
        let gone = Upstream::Gone {
            name: "origin/x".into(),
        };
        assert_eq!(gone.sync_words(), "upstream gone");
        assert_eq!(gone.name(), Some("origin/x"));
    }

    #[test]
    fn cd_command_is_shell_quoted() {
        assert_eq!(cd_command(Path::new("/c/my wt")), "cd '/c/my wt'");
        assert_eq!(cd_command(Path::new("/c/it's")), r"cd '/c/it'\''s'");
    }
}
