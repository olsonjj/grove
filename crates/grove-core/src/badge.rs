//! Worded worktree state badges. Every badge carries words, never color alone.

use crate::git::Upstream;
use crate::snapshot::{Presence, Worktree};

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
        }
    }

    /// Longer form for tooltips and accessibility labels.
    pub fn description(&self) -> String {
        match self {
            Self::MainWorktree => "The repo's original clone".into(),
            Self::Detached(sha) => format!("HEAD detached at {sha}"),
            Self::Missing => "Folder was deleted; run git worktree prune to clean up".into(),
            Self::Unavailable => "Its drive isn't mounted".into(),
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
        }
    }
}

impl Worktree {
    /// In display order: identity, presence, changes, sync, lock.
    pub fn badges(&self) -> Vec<Badge> {
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
                    if status.changed > 0 {
                        out.push(Badge::Changed(status.changed));
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
                }
            },
        }
        if self.locked {
            out.push(Badge::Locked);
        }
        out
    }
}
