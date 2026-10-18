//! What the details pane says about the selected row.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::git::{Upstream, WorktreeStatus};
use crate::launch::shell_quote;
use crate::snapshot::RepoSnapshot;
use crate::tree::RowId;

/// The details pane for one tree row, from `RepoSnapshot::details`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    /// Full branch name, worktree folder name, or folder prefix.
    pub title: String,
    /// E.g. "Branch · has worktree", "Worktree · detached HEAD".
    pub kind: String,
    /// Path identifying the worktree the pane's actions apply to: itself, or a branch's
    /// first (main first).
    pub worktree: Option<std::path::PathBuf>,
}

impl RepoSnapshot {
    /// Builds display details for a visible row, or `None` for a section header.
    pub fn details(&self, id: &RowId) -> Option<Details> {
        match id {
            RowId::Branch(name) => {
                let mut count = 0;
                let worktree = self
                    .worktrees
                    .iter()
                    .filter(|w| w.branch.as_deref() == Some(name))
                    .inspect(|_| count += 1)
                    .min_by(|a, b| b.is_main.cmp(&a.is_main).then(a.path.cmp(&b.path)))
                    .map(|w| w.path.clone());
                let kind = match count {
                    1 => "Branch · has worktree".to_string(),
                    n => format!("Branch · {n} worktrees"),
                };
                Some(Details {
                    title: name.clone(),
                    kind,
                    worktree,
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
                    worktree: Some(wt.path.clone()),
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
    /// Returns the configured upstream branch name, if any.
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

const fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
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

/// Total size in bytes of `path`'s regular files, walking the directory tree without
/// following symlinks (so aliased trees can't loop); unreadable entries count nothing,
/// so the total is a floor. `.git` is skipped: in a main worktree it's repo data, not
/// the working tree, and in a linked worktree it's a one-line file.
pub fn directory_size(path: &Path) -> u64 {
    fn walk(dir: &Path, total: &mut u64) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            // `entry.metadata` reads the link itself, never its target.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                walk(&entry.path(), total);
            } else if meta.is_file() {
                *total += meta.len();
            }
        }
    }
    let mut total = 0;
    if path.file_name().is_some_and(|n| n == ".git") {
        return 0;
    }
    walk(path, &mut total);
    total
}

/// A directory size in words: "512 B", "1.5 kB", "452 MB", at most one decimal, binary
/// units (1 kB = 1024 B). Beyond one megabyte, whole units; a gigabyte is Grove's ceiling.
pub fn size_words(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * KIB;
    let b = bytes as f64;
    let (n, unit) = if bytes < KIB as u64 {
        return format!("{bytes} B");
    } else if bytes < MIB as u64 {
        (b / KIB, "kB")
    } else if b < MIB * KIB {
        (b / MIB, "MB")
    } else {
        (b / (MIB * KIB), "GB")
    };
    if n < 10. {
        let mut one = format!("{n:.1}");
        if one.ends_with(".0") {
            one.truncate(one.len() - 2);
        }
        format!("{one} {unit}")
    } else {
        format!("{} {unit}", n.round())
    }
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
    fn size_words_rounds_to_the_nearest_named_unit() {
        const K: u64 = 1024;
        const M: u64 = K * K;
        let cases = [
            (0, "0 B"),
            (512, "512 B"),
            (1023, "1023 B"),
            (K, "1 kB"),
            (1536, "1.5 kB"),
            (10 * K, "10 kB"),
            (10486, "10 kB"),
            (M, "1 MB"),
            (M * 5 / 2, "2.5 MB"),
            (452 * M, "452 MB"),
            (M * 10 + K * 200, "10 MB"),
            (1024 * M, "1 GB"),
            (3 * 1024 * M, "3 GB"),
        ];
        for (bytes, want) in cases {
            assert_eq!(size_words(bytes), want, "{bytes} bytes");
        }
    }

    #[test]
    fn directory_size_sums_regular_files_without_following_links() {
        let tmp = std::env::temp_dir().join(format!("grove-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let nested = tmp.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.join("one"), vec![0u8; 100]).unwrap();
        std::fs::write(tmp.join("a").join("two"), vec![0u8; 200]).unwrap();
        std::fs::write(nested.join("three"), vec![0u8; 30]).unwrap();
        // A `.git` dir is skipped: repo data, not the working tree.
        std::fs::create_dir(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join(".git").join("packed"), vec![0u8; 9999]).unwrap();
        // A symlink counts its own nothing: not the target's bytes, no cycle.
        #[cfg(unix)]
        std::os::unix::fs::symlink(nested.join("three"), tmp.join("link")).unwrap();
        assert_eq!(directory_size(&tmp), 330);
        #[cfg(unix)]
        {
            std::fs::remove_file(nested.join("three")).unwrap();
            assert_eq!(directory_size(&tmp), 300);
        }

        // A missing dir reads as empty.
        assert_eq!(directory_size(&tmp.join("nope")), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn cd_command_is_shell_quoted() {
        assert_eq!(cd_command(Path::new("/c/my wt")), "cd '/c/my wt'");
        assert_eq!(cd_command(Path::new("/c/it's")), r"cd '/c/it'\''s'");
    }
}
