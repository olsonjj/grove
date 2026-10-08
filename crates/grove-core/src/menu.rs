//! Menu bar popover rows: every worktree grouped by repo, each with a one-line state.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::badge::Badge;
use crate::order::repo_order;
use crate::settings::SortMode;
use crate::snapshot::{Presence, RepoSnapshot, Snapshot, Worktree};
use crate::tree::by_name;

/// One repo's worktrees in the popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuGroup {
    /// The repo's display name.
    pub repo: String,
    pub rows: Vec<MenuRow>,
}

/// One worktree in the popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRow {
    pub path: PathBuf,
    /// The branch, or the folder name when detached.
    pub name: String,
    pub presence: Presence,
    pub state: StateLine,
    /// The most recently opened worktree.
    pub last_opened: bool,
}

/// A worktree's state in a few words: "3 changed · ↑2 ↓1", "main worktree · clean".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLine {
    pub text: String,
    /// Where `text` says "N changed", for the accent color (the words say it too).
    pub changed: Option<Range<usize>>,
}

/// Every repo with worktrees, in `repo_order` (`pinned` first, the rest by `repo_sort`).
/// Within a repo: the main worktree, then branches alphabetically, then detached worktrees
/// by path. `recents[0]` is marked `last_opened`.
pub fn menu(
    snapshot: &Snapshot,
    pinned: &[PathBuf],
    repo_sort: SortMode,
    recents: &[PathBuf],
) -> Vec<MenuGroup> {
    let last = recents.first().map(PathBuf::as_path);
    repo_order(snapshot, pinned, repo_sort)
        .iter()
        .filter(|r| !r.worktrees.is_empty())
        .map(|repo| {
            let mut wts: Vec<&Worktree> = repo.worktrees.iter().collect();
            wts.sort_by(|a, b| {
                b.is_main
                    .cmp(&a.is_main)
                    .then(a.branch.is_none().cmp(&b.branch.is_none()))
                    .then_with(|| match (&a.branch, &b.branch) {
                        (Some(x), Some(y)) => by_name(x, y),
                        _ => a.path.cmp(&b.path),
                    })
            });
            MenuGroup {
                repo: repo.display_name.clone(),
                rows: wts.into_iter().map(|wt| row(repo, wt, last)).collect(),
            }
        })
        .collect()
}

fn row(repo: &RepoSnapshot, wt: &Worktree, last: Option<&Path>) -> MenuRow {
    let name = wt.branch.clone().unwrap_or_else(|| {
        wt.path.file_name().map_or_else(
            || wt.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    });
    MenuRow {
        path: wt.path.clone(),
        name,
        presence: wt.presence,
        state: repo.state_line(wt),
        last_opened: last == Some(wt.path.as_path()),
    }
}

const SEP: &str = " · ";

impl RepoSnapshot {
    /// `wt`'s badges as one line, with "clean" when its status read found no changes
    /// (not when detached: the SHA says where it is) and "checking status…" until read.
    pub fn state_line(&self, wt: &Worktree) -> StateLine {
        let badges = self.badges(wt);
        let present = wt.presence == Presence::Present;
        let clean = present
            && matches!(wt.status, Some(Ok(_)))
            && !badges
                .iter()
                .any(|b| matches!(b, Badge::Detached(_) | Badge::Changed(_)));
        let mut parts: Vec<String> = badges
            .iter()
            .map(|b| match b {
                Badge::Missing | Badge::Unavailable => {
                    wt.presence.reason().unwrap_or_default().into()
                }
                b => b.label(),
            })
            .collect();
        if clean {
            parts.insert(usize::from(wt.is_main), "clean".into());
        }
        if present && wt.status.is_none() {
            parts.push("checking status…".into());
        }
        // `clean` excludes changes, so its insert doesn't shift this.
        let changed = badges
            .iter()
            .position(|b| matches!(b, Badge::Changed(_)))
            .map(|i| {
                let start: usize = parts[..i].iter().map(|p| p.len() + SEP.len()).sum();
                start..start + parts[i].len()
            });
        let text = parts.join(SEP);
        StateLine { text, changed }
    }
}
