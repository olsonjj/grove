//! Worded worktree state badges. Every badge carries words, never color alone.

use crate::git::{GitError, Upstream};
use crate::snapshot::{Presence, RepoSnapshot, Worktree};

/// One fact about a worktree, as a short label (`label`) and a sentence (`description`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Badge {
    MainWorktree,
    /// Short SHA.
    Detached(String),
    Missing,
    Unavailable,
    Changed(usize),
    /// Ahead/behind its upstream; never both zero.
    Sync {
        ahead: u32,
        behind: u32,
        upstream: String,
    },
    UpstreamGone(String),
    LocalOnly,
    Locked,
    /// `git status` failed or timed out.
    StatusFailed(String),
    /// The repo's last fetch failed (the error), so sync state may be out of date.
    Stale(String),
}

impl Badge {
    pub fn label(&self) -> String {
        match self {
            Self::MainWorktree => "main worktree".into(),
            Self::Detached(sha) => format!("detached @ {sha}"),
            Self::Missing => "missing".into(),
            Self::Unavailable => "unavailable".into(),
            Self::Changed(n) => format!("{n} changed"),
            Self::Sync { ahead, behind, .. } => match (ahead, behind) {
                (a, 0) => format!("↑{a}"),
                (0, b) => format!("↓{b}"),
                (a, b) => format!("↑{a} ↓{b}"),
            },
            Self::UpstreamGone(_) => "upstream gone".into(),
            Self::LocalOnly => "local only".into(),
            Self::Locked => "locked".into(),
            Self::StatusFailed(_) => "status failed".into(),
            Self::Stale(_) => "stale".into(),
        }
    }

    /// Longer form for tooltips and accessibility labels.
    pub fn description(&self) -> String {
        match self {
            Self::MainWorktree => "The repo's original clone".into(),
            Self::Detached(sha) => format!("HEAD detached at {sha}"),
            Self::Missing => capitalized(Presence::Missing.reason().unwrap_or_default()),
            Self::Unavailable => capitalized(Presence::Unavailable.reason().unwrap_or_default()),
            Self::Changed(1) => "1 uncommitted change".into(),
            Self::Changed(n) => format!("{n} uncommitted changes"),
            Self::Sync {
                ahead,
                behind,
                upstream,
            } => match (ahead, behind) {
                (a, 0) => format!("{a} ahead of {upstream}"),
                (0, b) => format!("{b} behind {upstream}"),
                (a, b) => format!("{a} ahead, {b} behind {upstream}"),
            },
            Self::UpstreamGone(name) => format!("Upstream {name} no longer exists"),
            Self::LocalOnly => "No upstream branch".into(),
            Self::Locked => "Locked with git worktree lock".into(),
            Self::StatusFailed(e) => e.clone(),
            Self::Stale(e) => format!("Last fetch failed, so sync state may be out of date: {e}"),
        }
    }
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

impl RepoSnapshot {
    /// `wt`'s badges, plus "stale" after its sync state if this repo's last fetch failed.
    pub fn badges(&self, wt: &Worktree) -> Vec<Badge> {
        wt.badges_with(self.fetch_error.as_ref())
    }
}

impl Worktree {
    /// In display order: identity, presence, changes, sync, stale, lock.
    pub(crate) fn badges_with(&self, fetch_error: Option<&GitError>) -> Vec<Badge> {
        let mut out = Vec::new();
        if self.is_main {
            out.push(Badge::MainWorktree);
        }
        if self.branch.is_none()
            && let Some(head) = &self.head
        {
            out.push(Badge::Detached(head.get(..7).unwrap_or(head).to_string()));
        }
        match self.presence {
            Presence::Missing => out.push(Badge::Missing),
            Presence::Unavailable => out.push(Badge::Unavailable),
            Presence::Present => match &self.status {
                None => {}
                Some(Err(e)) => out.push(Badge::StatusFailed(e.to_string())),
                Some(Ok(status)) => {
                    if status.changed() > 0 {
                        out.push(Badge::Changed(status.changed()));
                    }
                    match &status.upstream {
                        Upstream::Tracking {
                            name,
                            ahead,
                            behind,
                        } if *ahead > 0 || *behind > 0 => out.push(Badge::Sync {
                            ahead: *ahead,
                            behind: *behind,
                            upstream: name.clone(),
                        }),
                        Upstream::Tracking { .. } => {}
                        Upstream::Gone { name } => out.push(Badge::UpstreamGone(name.clone())),
                        Upstream::None if self.branch.is_some() => out.push(Badge::LocalOnly),
                        Upstream::None => {}
                    }
                    if let Some(e) = fetch_error
                        && !matches!(status.upstream, Upstream::None)
                    {
                        out.push(Badge::Stale(e.brief()));
                    }
                }
            },
        }
        if self.locked {
            out.push(Badge::Locked);
        }
        out
    }
}
