//! Menu bar popover rows: every worktree grouped by repo, each with a one-line state.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::badge::Badge;
use crate::order::repo_order;
use crate::settings::Settings;
use crate::snapshot::{Presence, RepoSnapshot, Snapshot, Worktree};
use crate::tree::RowKind;

/// One repo's worktrees in the popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuGroup {
    /// The repo's display name.
    pub repo: String,
    /// Worktrees shown under the repository.
    pub rows: Vec<MenuRow>,
}

/// One worktree in the popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRow {
    /// Absolute worktree path.
    pub path: PathBuf,
    /// The branch, or the folder name when detached.
    pub name: String,
    /// Whether the worktree folder can be opened.
    pub presence: Presence,
    /// Compact worktree state text.
    pub state: StateLine,
    /// The most recently opened worktree.
    pub last_opened: bool,
}

/// A worktree's state in a few words: "3 changed · ↑2 ↓1", "main worktree · clean".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLine {
    /// Complete state text.
    pub text: String,
    /// Where `text` says "N changed", for the accent color (the words say it too).
    pub changed: Option<Range<usize>>,
}

/// Every repo with worktrees, in `repo_order` (pinned first, the rest by `repo_sort`).
/// Within a repo, worktrees are in the tree's order under `tree_sort`, folders expanded.
/// `recents[0]` is marked `last_opened`.
/// Hiding is display-only for the desktop sidebar, so the menu-bar popover passes an empty
/// hidden set and lists every repo.
pub fn menu(snapshot: &Snapshot, settings: &Settings) -> Vec<MenuGroup> {
    let last = settings.recents.first().map(PathBuf::as_path);
    let last_opened = &settings.last_opened;
    repo_order(
        snapshot,
        &settings.pinned_repos,
        &[],
        last_opened,
        settings.repo_sort,
    )
    .iter()
    .filter(|r| !r.worktrees.is_empty())
    .map(|repo| {
        let tree = repo.tree(&[], settings.tree_sort, last_opened);
        let rows = tree
            .rows
            .iter()
            .filter_map(|r| match &r.kind {
                RowKind::Worktree { path, .. } => repo.worktrees.iter().find(|w| &w.path == path),
                _ => None,
            })
            .map(|wt| row(repo, wt, last))
            .collect();
        MenuGroup {
            repo: repo.display_name.clone(),
            rows,
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
